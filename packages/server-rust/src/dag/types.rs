use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// All Result types in trait signatures use anyhow::Result.
use anyhow::Result;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// How items are routed from a source processor to destination processors.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RoutingPolicy {
    /// Round-robin distribution to consumers.
    Unicast,
    /// Route by partition key extracted from the named field of the item.
    Partitioned { partition_key_field: String },
    /// Send to all consumers (broadcast).
    Broadcast,
    /// 1:1 mapping between source and dest processors.
    Isolated,
    /// Local round-robin, remote broadcast.
    Fanout,
}

/// Identifies which processor implementation to instantiate on a receiving node.
/// Used in `DagPlanDescriptor` / `VertexDescriptor` — NOT in runtime `Vertex`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProcessorType {
    Scan,
    Filter,
    Project,
    Aggregate,
    Combine,
    Collector,
    NetworkSender,
    NetworkReceiver,
    Sort,
    Limit,
}

// ---------------------------------------------------------------------------
// Serializable plan descriptor structs (wire transport)
// ---------------------------------------------------------------------------

/// Serializable description of a single vertex, used for plan distribution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VertexDescriptor {
    pub name: String,
    pub local_parallelism: u32,
    pub processor_type: ProcessorType,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub preferred_partitions: Option<Vec<u32>>,
    /// Processor-specific configuration (`map_name`, predicate bytes, field list, etc.).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub config: Option<rmpv::Value>,
}

/// Serializable description of a directed edge between two vertices.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Edge {
    pub source_name: String,
    pub source_ordinal: u32,
    pub dest_name: String,
    pub dest_ordinal: u32,
    pub routing_policy: RoutingPolicy,
    pub priority: u32,
}

/// Serializable DAG plan: vertices + edges. Sent over the wire via `DagExecutePayload`.
/// Receiving nodes reconstruct a runtime `Dag` (with `Vertex`/`ProcessorSupplier`) from this.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DagPlanDescriptor {
    pub vertices: Vec<VertexDescriptor>,
    pub edges: Vec<Edge>,
}

/// Query execution configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryConfig {
    pub timeout_ms: u64,
    pub memory_limit_bytes: u64,
    pub collect_metrics: bool,
}

impl Default for QueryConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 30_000,
            memory_limit_bytes: 64 * 1024 * 1024,
            collect_metrics: false,
        }
    }
}

/// Serializable execution plan: plan descriptor, partition assignments, metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionPlan {
    pub plan: DagPlanDescriptor,
    /// `node_id` -> `partition_ids` assigned to that node for this execution.
    pub partition_assignment: HashMap<String, Vec<u32>>,
    pub version: u64,
    pub config: QueryConfig,
    pub created_at: i64,
}

// ---------------------------------------------------------------------------
// Traits
// ---------------------------------------------------------------------------

/// Context passed to each processor during initialization.
/// All fields are owned types, making Clone zero-cost.
/// Processors may store a copy from `init()` for later use.
#[derive(Debug, Clone)]
pub struct ProcessorContext {
    /// Unique identifier of the cluster node running this processor.
    pub node_id: String,
    pub global_processor_index: u32,
    pub local_processor_index: u32,
    pub total_parallelism: u32,
    pub vertex_name: String,
    /// Local `partition_ids` assigned to this processor instance.
    pub partition_ids: Vec<u32>,
}

/// Single-threaded execution unit. Processes items from inbox, emits to outbox.
/// Processors are `Send` but not required to be `Sync` — the executor holds them
/// behind exclusive access and never shares them across threads.
pub trait Processor: Send {
    /// Initialize with context. Called once before processing begins.
    ///
    /// # Errors
    /// Returns an error if the processor cannot initialize (e.g., invalid config).
    fn init(&mut self, context: &ProcessorContext) -> Result<()>;

    /// Process items from inbox at the given ordinal. Returns `true` when done.
    ///
    /// # Errors
    /// Returns an error if processing fails (e.g., malformed input data).
    fn process(
        &mut self,
        ordinal: u32,
        inbox: &mut dyn Inbox,
        outbox: &mut dyn Outbox,
    ) -> Result<bool>;

    /// Called when all upstream inputs are complete. Returns `true` when done emitting.
    ///
    /// # Errors
    /// Returns an error if final emission fails.
    fn complete(&mut self, outbox: &mut dyn Outbox) -> Result<bool>;

    /// Whether this processor can share a tokio task with others (non-blocking).
    fn is_cooperative(&self) -> bool;

    /// Release any held resources.
    fn close(&mut self);
}

/// Factory for creating `Processor` instances on each node.
/// `Send + Sync` so it can be shared across threads during plan distribution.
pub trait ProcessorSupplier: Send + Sync {
    /// Create `count` processor instances for local execution.
    fn get(&self, count: u32) -> Vec<Box<dyn Processor>>;

    /// Produce a heap-allocated clone of this supplier (used when building
    /// a `Dag` from a `DagPlanDescriptor` on the receiving node).
    fn clone_supplier(&self) -> Box<dyn ProcessorSupplier>;
}

/// Read-side buffer for processor input.
pub trait Inbox: Send {
    fn is_empty(&self) -> bool;
    fn peek(&self) -> Option<&rmpv::Value>;
    fn poll(&mut self) -> Option<rmpv::Value>;
    fn drain(&mut self, callback: &mut dyn FnMut(rmpv::Value));
    fn len(&self) -> usize;
}

/// Write-side buffer for processor output.
pub trait Outbox: Send {
    /// Offer an item to a specific ordinal. Returns `false` if backpressure applies.
    fn offer(&mut self, ordinal: u32, item: rmpv::Value) -> bool;
    /// Offer an item to all ordinals.
    fn offer_to_all(&mut self, item: rmpv::Value) -> bool;
    /// Check whether the given ordinal has capacity.
    fn has_capacity(&self, ordinal: u32) -> bool;
    /// Number of output ordinals.
    fn bucket_count(&self) -> u32;
}

// ---------------------------------------------------------------------------
// Runtime-only structs (no Serialize/Deserialize — contain trait objects)
// ---------------------------------------------------------------------------

/// Runtime vertex: name, parallelism, preferred partitions, and a supplier
/// that creates `Processor` instances. Use `VertexDescriptor` for wire transport.
pub struct Vertex {
    pub name: String,
    pub local_parallelism: u32,
    pub processor_supplier: Box<dyn ProcessorSupplier>,
    pub preferred_partitions: Option<Vec<u32>>,
}

impl std::fmt::Debug for Vertex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vertex")
            .field("name", &self.name)
            .field("local_parallelism", &self.local_parallelism)
            .field("preferred_partitions", &self.preferred_partitions)
            .finish_non_exhaustive()
    }
}

/// Runtime DAG: vertices indexed by name, edges, and adjacency indexes.
/// Not serializable — use `DagPlanDescriptor` for wire transport.
pub struct Dag {
    vertices: HashMap<String, Vertex>,
    edges: Vec<Edge>,
    /// Edge indexes keyed by source vertex name (into `edges` vec).
    edges_by_source: HashMap<String, Vec<usize>>,
    /// Edge indexes keyed by destination vertex name (into `edges` vec).
    edges_by_dest: HashMap<String, Vec<usize>>,
}

impl Dag {
    /// Create an empty DAG.
    #[must_use]
    pub fn new() -> Self {
        Self {
            vertices: HashMap::new(),
            edges: Vec::new(),
            edges_by_source: HashMap::new(),
            edges_by_dest: HashMap::new(),
        }
    }

    /// Add a vertex to the DAG.
    pub fn new_vertex(&mut self, vertex: Vertex) {
        self.vertices.insert(vertex.name.clone(), vertex);
    }

    /// Add a directed edge. Both source and dest must be added via `new_vertex`
    /// before calling `validate()`, but the edge may be added in any order.
    pub fn edge(&mut self, edge: Edge) {
        let idx = self.edges.len();
        self.edges_by_source
            .entry(edge.source_name.clone())
            .or_default()
            .push(idx);
        self.edges_by_dest
            .entry(edge.dest_name.clone())
            .or_default()
            .push(idx);
        self.edges.push(edge);
    }

    /// Validate the DAG and return vertices in topological order (Kahn's algorithm).
    ///
    /// # Errors
    /// Returns an error if any edge references a vertex name not present in the
    /// vertex set, or if a cycle is detected.
    pub fn validate(&self) -> Result<Vec<&Vertex>> {
        // Step 1: Edge validity — all referenced vertex names must exist.
        for edge in &self.edges {
            if !self.vertices.contains_key(&edge.source_name) {
                return Err(anyhow::anyhow!(
                    "edge references unknown source vertex '{}'",
                    edge.source_name
                ));
            }
            if !self.vertices.contains_key(&edge.dest_name) {
                return Err(anyhow::anyhow!(
                    "edge references unknown destination vertex '{}'",
                    edge.dest_name
                ));
            }
        }

        // Step 2: Kahn's algorithm for topological sort + cycle detection.
        // Build in-degree map.
        let mut in_degree: HashMap<&str, usize> = self
            .vertices
            .keys()
            .map(|k| (k.as_str(), 0usize))
            .collect();

        for edge in &self.edges {
            *in_degree.entry(edge.dest_name.as_str()).or_insert(0) += 1;
        }

        // Seed queue with zero-in-degree vertices.
        let mut queue: std::collections::VecDeque<&str> = in_degree
            .iter()
            .filter_map(|(&name, &deg)| if deg == 0 { Some(name) } else { None })
            .collect();

        let mut sorted: Vec<&Vertex> = Vec::with_capacity(self.vertices.len());

        while let Some(name) = queue.pop_front() {
            let vertex = self
                .vertices
                .get(name)
                .ok_or_else(|| anyhow::anyhow!("vertex '{name}' must exist in vertex set"))?;
            sorted.push(vertex);

            // Decrement in-degree for all neighbors.
            if let Some(idxs) = self.edges_by_source.get(name) {
                for &idx in idxs {
                    let dest = self.edges[idx].dest_name.as_str();
                    let deg = in_degree.entry(dest).or_insert(0);
                    *deg = deg.saturating_sub(1);
                    if *deg == 0 {
                        queue.push_back(dest);
                    }
                }
            }
        }

        if sorted.len() != self.vertices.len() {
            return Err(anyhow::anyhow!(
                "DAG contains a cycle: {} of {} vertices could not be sorted",
                self.vertices.len() - sorted.len(),
                self.vertices.len()
            ));
        }

        Ok(sorted)
    }

    /// Return a reference to all vertices keyed by name.
    #[must_use]
    pub fn get_vertices(&self) -> &HashMap<String, Vertex> {
        &self.vertices
    }

    /// Return all edges.
    #[must_use]
    pub fn get_edges(&self) -> &[Edge] {
        &self.edges
    }

    /// Return all edges whose source vertex name matches `name`.
    #[must_use]
    pub fn edges_for_source(&self, name: &str) -> Vec<&Edge> {
        self.edges_by_source
            .get(name)
            .map(|idxs| idxs.iter().map(|&i| &self.edges[i]).collect())
            .unwrap_or_default()
    }

    /// Return all edges whose destination vertex name matches `name`.
    #[must_use]
    pub fn edges_for_dest(&self, name: &str) -> Vec<&Edge> {
        self.edges_by_dest
            .get(name)
            .map(|idxs| idxs.iter().map(|&i| &self.edges[i]).collect())
            .unwrap_or_default()
    }

    /// Reconstruct a runtime `Dag` from a serializable `DagPlanDescriptor`.
    ///
    /// The `supplier_factory` callback is responsible for mapping each
    /// `VertexDescriptor` to a concrete `ProcessorSupplier`. Tests may supply a
    /// stub closure; the real factory is provided by the executor module.
    ///
    /// # Errors
    /// Returns an error if the `supplier_factory` fails for any vertex descriptor.
    pub fn from_descriptor(
        desc: &DagPlanDescriptor,
        supplier_factory: &dyn Fn(&VertexDescriptor) -> Result<Box<dyn ProcessorSupplier>>,
    ) -> Result<Dag> {
        let mut dag = Dag::new();

        for vd in &desc.vertices {
            let supplier = supplier_factory(vd)?;
            dag.new_vertex(Vertex {
                name: vd.name.clone(),
                local_parallelism: vd.local_parallelism,
                processor_supplier: supplier,
                preferred_partitions: vd.preferred_partitions.clone(),
            });
        }

        for edge in &desc.edges {
            dag.edge(edge.clone());
        }

        Ok(dag)
    }
}

impl Default for Dag {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- Minimal stub implementations for tests ---

    struct StubProcessor;

    impl Processor for StubProcessor {
        fn init(&mut self, _context: &ProcessorContext) -> Result<()> {
            Ok(())
        }

        fn process(
            &mut self,
            _ordinal: u32,
            _inbox: &mut dyn Inbox,
            _outbox: &mut dyn Outbox,
        ) -> Result<bool> {
            Ok(true)
        }

        fn complete(&mut self, _outbox: &mut dyn Outbox) -> Result<bool> {
            Ok(true)
        }

        fn is_cooperative(&self) -> bool {
            true
        }

        fn close(&mut self) {}
    }

    struct StubSupplier;

    impl ProcessorSupplier for StubSupplier {
        fn get(&self, count: u32) -> Vec<Box<dyn Processor>> {
            (0..count).map(|_| Box::new(StubProcessor) as Box<dyn Processor>).collect()
        }

        fn clone_supplier(&self) -> Box<dyn ProcessorSupplier> {
            Box::new(StubSupplier)
        }
    }

    fn make_vertex(name: &str) -> Vertex {
        Vertex {
            name: name.to_string(),
            local_parallelism: 1,
            processor_supplier: Box::new(StubSupplier),
            preferred_partitions: None,
        }
    }

    fn make_edge(src: &str, dst: &str) -> Edge {
        Edge {
            source_name: src.to_string(),
            source_ordinal: 0,
            dest_name: dst.to_string(),
            dest_ordinal: 0,
            routing_policy: RoutingPolicy::Unicast,
            priority: 0,
        }
    }

    // --- AC #1: DAG validates correctly ---

    #[test]
    fn valid_dag_returns_topological_order() {
        // A -> B -> C
        let mut dag = Dag::new();
        dag.new_vertex(make_vertex("A"));
        dag.new_vertex(make_vertex("B"));
        dag.new_vertex(make_vertex("C"));
        dag.edge(make_edge("A", "B"));
        dag.edge(make_edge("B", "C"));

        let sorted = dag.validate().expect("valid DAG should not error");
        assert_eq!(sorted.len(), 3);

        // Topological constraint: A before B, B before C.
        let pos: HashMap<&str, usize> = sorted
            .iter()
            .enumerate()
            .map(|(i, v)| (v.name.as_str(), i))
            .collect();
        assert!(pos["A"] < pos["B"]);
        assert!(pos["B"] < pos["C"]);
    }

    #[test]
    fn cyclic_dag_returns_error() {
        // A -> B -> C -> A
        let mut dag = Dag::new();
        dag.new_vertex(make_vertex("A"));
        dag.new_vertex(make_vertex("B"));
        dag.new_vertex(make_vertex("C"));
        dag.edge(make_edge("A", "B"));
        dag.edge(make_edge("B", "C"));
        dag.edge(make_edge("C", "A"));

        let err = dag.validate().unwrap_err();
        assert!(
            err.to_string().contains("cycle"),
            "error should mention cycle: {err}"
        );
    }

    #[test]
    fn edge_referencing_missing_vertex_returns_error() {
        let mut dag = Dag::new();
        dag.new_vertex(make_vertex("A"));
        dag.edge(make_edge("A", "MISSING"));

        let err = dag.validate().unwrap_err();
        assert!(
            err.to_string().contains("MISSING"),
            "error should mention the missing vertex name: {err}"
        );
    }

    // --- AC #2: RoutingPolicy variants round-trip through MsgPack ---

    fn routing_roundtrip(policy: RoutingPolicy) -> RoutingPolicy {
        let bytes = rmp_serde::to_vec_named(&policy).expect("serialize");
        rmp_serde::from_slice(&bytes).expect("deserialize")
    }

    #[test]
    fn routing_policy_unicast_roundtrip() {
        assert_eq!(routing_roundtrip(RoutingPolicy::Unicast), RoutingPolicy::Unicast);
    }

    #[test]
    fn routing_policy_partitioned_roundtrip() {
        let policy = RoutingPolicy::Partitioned {
            partition_key_field: "userId".to_string(),
        };
        assert_eq!(routing_roundtrip(policy.clone()), policy);
    }

    #[test]
    fn routing_policy_broadcast_roundtrip() {
        assert_eq!(routing_roundtrip(RoutingPolicy::Broadcast), RoutingPolicy::Broadcast);
    }

    #[test]
    fn routing_policy_isolated_roundtrip() {
        assert_eq!(routing_roundtrip(RoutingPolicy::Isolated), RoutingPolicy::Isolated);
    }

    #[test]
    fn routing_policy_fanout_roundtrip() {
        assert_eq!(routing_roundtrip(RoutingPolicy::Fanout), RoutingPolicy::Fanout);
    }

    // --- AC #3: Plan descriptors serialize correctly ---

    #[test]
    fn dag_plan_descriptor_roundtrip() {
        let desc = DagPlanDescriptor {
            vertices: vec![
                VertexDescriptor {
                    name: "scan".to_string(),
                    local_parallelism: 1,
                    processor_type: ProcessorType::Scan,
                    preferred_partitions: Some(vec![0, 1]),
                    config: None,
                },
                VertexDescriptor {
                    name: "collector".to_string(),
                    local_parallelism: 1,
                    processor_type: ProcessorType::Collector,
                    preferred_partitions: None,
                    config: None,
                },
            ],
            edges: vec![Edge {
                source_name: "scan".to_string(),
                source_ordinal: 0,
                dest_name: "collector".to_string(),
                dest_ordinal: 0,
                routing_policy: RoutingPolicy::Unicast,
                priority: 0,
            }],
        };

        let bytes = rmp_serde::to_vec_named(&desc).expect("serialize");
        let decoded: DagPlanDescriptor = rmp_serde::from_slice(&bytes).expect("deserialize");

        assert_eq!(decoded.vertices.len(), 2);
        assert_eq!(decoded.vertices[0].name, "scan");
        assert_eq!(decoded.vertices[1].name, "collector");
        assert_eq!(decoded.edges.len(), 1);
        assert_eq!(decoded.vertices[0].preferred_partitions, Some(vec![0, 1]));
    }

    #[test]
    fn execution_plan_roundtrip() {
        let plan = ExecutionPlan {
            plan: DagPlanDescriptor {
                vertices: vec![VertexDescriptor {
                    name: "scan".to_string(),
                    local_parallelism: 2,
                    processor_type: ProcessorType::Aggregate,
                    preferred_partitions: None,
                    config: None,
                }],
                edges: vec![],
            },
            partition_assignment: {
                let mut m = HashMap::new();
                m.insert("node-1".to_string(), vec![0, 1, 2]);
                m
            },
            version: 42,
            config: QueryConfig::default(),
            created_at: 1_700_000_000_000,
        };

        let bytes = rmp_serde::to_vec_named(&plan).expect("serialize");
        let decoded: ExecutionPlan = rmp_serde::from_slice(&bytes).expect("deserialize");

        assert_eq!(decoded.version, 42);
        assert_eq!(decoded.config.timeout_ms, 30_000);
        assert_eq!(decoded.config.memory_limit_bytes, 64 * 1024 * 1024);
        assert!(!decoded.config.collect_metrics);
        assert_eq!(decoded.partition_assignment["node-1"], vec![0, 1, 2]);
    }

    // --- AC #4: Dag::from_descriptor() reconstructs runtime DAG ---

    #[test]
    fn from_descriptor_reconstructs_dag() {
        let desc = DagPlanDescriptor {
            vertices: vec![
                VertexDescriptor {
                    name: "scan".to_string(),
                    local_parallelism: 1,
                    processor_type: ProcessorType::Scan,
                    preferred_partitions: None,
                    config: None,
                },
                VertexDescriptor {
                    name: "filter".to_string(),
                    local_parallelism: 1,
                    processor_type: ProcessorType::Filter,
                    preferred_partitions: None,
                    config: None,
                },
            ],
            edges: vec![Edge {
                source_name: "scan".to_string(),
                source_ordinal: 0,
                dest_name: "filter".to_string(),
                dest_ordinal: 0,
                routing_policy: RoutingPolicy::Unicast,
                priority: 0,
            }],
        };

        // Stub factory: returns a StubSupplier regardless of ProcessorType.
        let dag = Dag::from_descriptor(&desc, &|_vd| {
            Ok(Box::new(StubSupplier) as Box<dyn ProcessorSupplier>)
        })
        .expect("from_descriptor should succeed");

        assert_eq!(dag.get_vertices().len(), 2);
        assert!(dag.get_vertices().contains_key("scan"));
        assert!(dag.get_vertices().contains_key("filter"));
        assert_eq!(dag.get_edges().len(), 1);
        assert_eq!(dag.get_edges()[0].source_name, "scan");
        assert_eq!(dag.get_edges()[0].dest_name, "filter");

        // The reconstructed DAG should also pass validation.
        let sorted = dag.validate().expect("reconstructed DAG should be valid");
        assert_eq!(sorted.len(), 2);
    }

    // --- QueryConfig defaults ---

    #[test]
    fn query_config_defaults() {
        let cfg = QueryConfig::default();
        assert_eq!(cfg.timeout_ms, 30_000);
        assert_eq!(cfg.memory_limit_bytes, 64 * 1024 * 1024);
        assert!(!cfg.collect_metrics);
    }

    // --- ProcessorContext is Debug + Clone ---

    #[test]
    fn processor_context_clone_and_debug() {
        let ctx = ProcessorContext {
            node_id: "node-1".to_string(),
            global_processor_index: 0,
            local_processor_index: 0,
            total_parallelism: 4,
            vertex_name: "scan".to_string(),
            partition_ids: vec![0, 1],
        };
        let cloned = ctx.clone();
        assert_eq!(cloned.node_id, "node-1");
        let _ = format!("{ctx:?}");
    }
}
