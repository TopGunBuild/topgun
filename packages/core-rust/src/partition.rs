//! Partition hash and lookup for distributing data across cluster nodes.
//!
//! `TopGun` uses 271 hash-based partitions. The partition hash function
//! (`fnv1a_hash(key) % 271`) is the shared contract between the TS client,
//! TS server, and Rust server. This module provides:
//!
//! - [`PARTITION_COUNT`]: The number of partitions (271, a prime for uniform distribution)
//! - [`hash_to_partition`]: Maps a string key to a partition ID in `[0, 271)`
//! - [`PartitionLookup`]: Read-only trait for partition ownership queries
//! - [`PartitionTable`]: Versioned partition-to-owner lookup, populated from wire messages
//! - [`get_relevant_partitions`]: Partition pruning from query key predicates

use std::collections::HashSet;

use crate::hash::fnv1a_hash;
use crate::messages::base::{PredicateNode, PredicateOp, Query};
use crate::messages::cluster::PartitionMapPayload;

/// Number of partitions in the cluster. A prime chosen for uniform modulo distribution.
pub const PARTITION_COUNT: u32 = 271;

/// Key attributes that identify partition-routable fields.
const KEY_ATTRIBUTES: &[&str] = &["_key", "key", "id", "_id"];

// ---------------------------------------------------------------------------
// hash_to_partition
// ---------------------------------------------------------------------------

/// Compute partition ID for a given key.
///
/// Equivalent to TS `hashString(key) % PARTITION_COUNT`.
///
/// # Examples
///
/// ```
/// use topgun_core::partition::hash_to_partition;
///
/// assert_eq!(hash_to_partition("hello"), 95);
/// assert_eq!(hash_to_partition("key1"), 268);
/// ```
#[must_use]
pub fn hash_to_partition(key: &str) -> u32 {
    fnv1a_hash(key) % PARTITION_COUNT
}

// ---------------------------------------------------------------------------
// PartitionLookup trait
// ---------------------------------------------------------------------------

/// Read-only partition ownership queries.
pub trait PartitionLookup {
    /// Get the owner node ID for a partition, if assigned.
    fn get_owner(&self, partition_id: u32) -> Option<&str>;

    /// Get the partition table version.
    fn version(&self) -> u32;

    /// Total partition count.
    fn partition_count(&self) -> u32;
}

// ---------------------------------------------------------------------------
// PartitionTable
// ---------------------------------------------------------------------------

/// Versioned partition-to-owner lookup table.
///
/// Uses a `Vec<Option<String>>` indexed by partition ID for O(1) lookup,
/// since partition IDs are dense integers in `[0, 271)`.
pub struct PartitionTable {
    /// Indexed by partition ID (length = 271). Each slot holds the owner node ID, or `None`.
    owners: Vec<Option<String>>,
    version: u32,
}

impl PartitionTable {
    /// Creates an empty table with all partitions unassigned and version 0.
    #[must_use]
    pub fn new() -> Self {
        Self {
            owners: vec![None; PARTITION_COUNT as usize],
            version: 0,
        }
    }

    /// Populates a table from a wire-format `PartitionMapPayload`.
    #[must_use]
    pub fn from_payload(payload: &PartitionMapPayload) -> Self {
        let mut table = Self {
            owners: vec![None; PARTITION_COUNT as usize],
            version: payload.version,
        };
        for p in &payload.partitions {
            if (p.partition_id as usize) < table.owners.len() {
                table.owners[p.partition_id as usize] = Some(p.owner_node_id.clone());
            }
        }
        table
    }

    /// Assign an owner to a partition.
    ///
    /// # Panics
    ///
    /// Panics if `partition_id >= PARTITION_COUNT`.
    pub fn set_owner(&mut self, partition_id: u32, node_id: String) {
        assert!(
            partition_id < PARTITION_COUNT,
            "partition_id {partition_id} out of range [0, {PARTITION_COUNT})"
        );
        self.owners[partition_id as usize] = Some(node_id);
    }

    /// Combines hash + lookup: returns the owner of the partition for a given key.
    #[must_use]
    pub fn get_owner_for_key(&self, key: &str) -> Option<&str> {
        let pid = hash_to_partition(key);
        self.get_owner(pid)
    }

    /// Returns a deduplicated list of owner node IDs for the given partition IDs.
    /// Unassigned partitions are excluded.
    #[must_use]
    pub fn owner_nodes_for_partitions(&self, partition_ids: &[u32]) -> Vec<&str> {
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        for &pid in partition_ids {
            if let Some(owner) = self.get_owner(pid) {
                if seen.insert(owner) {
                    result.push(owner);
                }
            }
        }
        result
    }

    /// Returns all partition IDs owned by the given node.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)] // Safe: PARTITION_COUNT is 271, well within u32
    pub fn partitions_for_node(&self, node_id: &str) -> Vec<u32> {
        self.owners
            .iter()
            .enumerate()
            .filter_map(|(i, owner)| {
                owner
                    .as_deref()
                    .filter(|&o| o == node_id)
                    .map(|_| i as u32)
            })
            .collect()
    }
}

impl Default for PartitionTable {
    fn default() -> Self {
        Self::new()
    }
}

impl PartitionLookup for PartitionTable {
    fn get_owner(&self, partition_id: u32) -> Option<&str> {
        self.owners
            .get(partition_id as usize)
            .and_then(|o| o.as_deref())
    }

    fn version(&self) -> u32 {
        self.version
    }

    fn partition_count(&self) -> u32 {
        PARTITION_COUNT
    }
}

// ---------------------------------------------------------------------------
// Partition Pruning
// ---------------------------------------------------------------------------

/// Extract relevant partition IDs from a `Query`.
///
/// Returns `None` when pruning is not possible (no key filter, OR/NOT predicates).
/// Returns `Some(Vec<u32>)` with deduplicated, sorted partition IDs when key values
/// can be extracted.
#[must_use]
pub fn get_relevant_partitions(query: &Query) -> Option<Vec<u32>> {
    // Try the where clause first
    if let Some(ref where_clause) = query.r#where {
        if let Some(keys) = extract_keys_from_where(where_clause) {
            let mut pids: Vec<u32> = keys.iter().map(|k| hash_to_partition(k)).collect();
            pids.sort_unstable();
            pids.dedup();
            return Some(pids);
        }
    }

    // Then try the predicate tree
    if let Some(ref predicate) = query.predicate {
        if let Some(keys) = extract_keys_from_predicate(predicate) {
            let mut pids: Vec<u32> = keys.iter().map(|k| hash_to_partition(k)).collect();
            pids.sort_unstable();
            pids.dedup();
            return Some(pids);
        }
    }

    None
}

/// Extract key values from a where clause.
///
/// Checks `KEY_ATTRIBUTES` for:
/// - Direct string/integer value: `{ "_key": "hello" }`
/// - Array of values: `{ "_key": ["a", "b"] }`
/// - `$eq` operator: `{ "_key": { "$eq": "hello" } }`
/// - `$in` operator: `{ "_key": { "$in": ["a", "b"] } }`
fn extract_keys_from_where(
    where_clause: &std::collections::HashMap<String, rmpv::Value>,
) -> Option<Vec<String>> {
    for attr in KEY_ATTRIBUTES {
        if let Some(val) = where_clause.get(*attr) {
            return extract_key_values_from_rmpv(val);
        }
    }
    None
}

/// Extract string key values from an `rmpv::Value`.
///
/// Handles:
/// - String value -> single key
/// - Integer value -> stringified key
/// - Array of strings/integers -> multiple keys
/// - Map with "$eq" -> single key
/// - Map with "$in" -> multiple keys
fn extract_key_values_from_rmpv(val: &rmpv::Value) -> Option<Vec<String>> {
    match val {
        rmpv::Value::String(s) => s.as_str().map(|s| vec![s.to_string()]),
        rmpv::Value::Integer(i) => {
            // Match TS behavior: typeof value === 'number'
            if let Some(n) = i.as_i64() {
                Some(vec![n.to_string()])
            } else {
                i.as_u64().map(|n| vec![n.to_string()])
            }
        }
        rmpv::Value::Array(arr) => {
            let keys: Vec<String> = arr
                .iter()
                .filter_map(value_to_string)
                .collect();
            if keys.is_empty() {
                None
            } else {
                Some(keys)
            }
        }
        rmpv::Value::Map(entries) => {
            // Look for $eq or $in operators
            for (k, v) in entries {
                if let Some(key_str) = k.as_str() {
                    if key_str == "$eq" {
                        return value_to_string(v).map(|s| vec![s]);
                    }
                    if key_str == "$in" {
                        if let rmpv::Value::Array(arr) = v {
                            let keys: Vec<String> =
                                arr.iter().filter_map(value_to_string).collect();
                            if !keys.is_empty() {
                                return Some(keys);
                            }
                        }
                    }
                }
            }
            None
        }
        _ => None,
    }
}

/// Convert an `rmpv::Value` to a string key (string or stringified integer).
fn value_to_string(val: &rmpv::Value) -> Option<String> {
    match val {
        rmpv::Value::String(s) => s.as_str().map(std::string::ToString::to_string),
        rmpv::Value::Integer(i) => {
            if let Some(n) = i.as_i64() {
                Some(n.to_string())
            } else {
                i.as_u64().map(|n| n.to_string())
            }
        }
        _ => None,
    }
}

/// Extract key values from a predicate tree.
///
/// Supports:
/// - `eq` on key attributes -> single key
/// - `and` with children -> merges keys from all children
/// - `or` / `not` -> returns `None` (cannot prune)
fn extract_keys_from_predicate(predicate: &PredicateNode) -> Option<Vec<String>> {
    match predicate.op {
        PredicateOp::Eq => {
            // Check if the attribute is a key attribute
            if let Some(ref attr) = predicate.attribute {
                if KEY_ATTRIBUTES.contains(&attr.as_str()) {
                    if let Some(ref val) = predicate.value {
                        return value_to_string(val).map(|s| vec![s]);
                    }
                }
            }
            None
        }
        PredicateOp::And => {
            // Merge keys from all children that produce keys
            if let Some(ref children) = predicate.children {
                let mut all_keys = Vec::new();
                for child in children {
                    if let Some(keys) = extract_keys_from_predicate(child) {
                        all_keys.extend(keys);
                    }
                }
                if all_keys.is_empty() {
                    None
                } else {
                    Some(all_keys)
                }
            } else {
                None
            }
        }
        // or/not predicates cannot be pruned; other ops (gt, lt, etc.) cannot
        // determine exact partition IDs either
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::messages::cluster::{NodeEndpoints, NodeInfo, NodeStatus, PartitionInfo};

    // ---- AC-1: Hash compatibility ----

    #[test]
    fn ac1_hash_to_partition_hello() {
        // fnv1a_hash("hello") = 1_335_831_723, 1_335_831_723 % 271 = 95
        assert_eq!(hash_to_partition("hello"), 95);
        assert_eq!(fnv1a_hash("hello") % PARTITION_COUNT, 95);
    }

    // ---- AC-2: Partition range ----

    #[test]
    fn ac2_partition_range_10000_keys() {
        for i in 0..10_000 {
            let key = format!("random-key-{i}");
            let pid = hash_to_partition(&key);
            assert!(pid < PARTITION_COUNT, "partition {pid} out of range for key '{key}'");
        }
    }

    // ---- AC-3: Cross-language test vectors ----

    #[test]
    fn ac3_cross_language_key1() {
        // fnv1a_hash("key1") = 927_623_783, 927_623_783 % 271 = 268
        assert_eq!(hash_to_partition("key1"), 268);
    }

    #[test]
    fn ac3_cross_language_empty() {
        // fnv1a_hash("") = 2_166_136_261, 2_166_136_261 % 271 = 199
        assert_eq!(hash_to_partition(""), 199);
    }

    #[test]
    fn ac3_cross_language_user_alice() {
        // fnv1a_hash("user:alice") = 927_278_352, 927_278_352 % 271 = 91
        assert_eq!(hash_to_partition("user:alice"), 91);
    }

    // ---- AC-4: PartitionTable from_payload ----

    fn make_test_payload(node_count: usize) -> PartitionMapPayload {
        let nodes: Vec<NodeInfo> = (0..node_count)
            .map(|i| NodeInfo {
                node_id: format!("node-{i}"),
                endpoints: NodeEndpoints {
                    websocket: format!("ws://node-{i}:8080"),
                    http: None,
                },
                status: NodeStatus::ACTIVE,
            })
            .collect();

        let partitions: Vec<PartitionInfo> = (0..PARTITION_COUNT)
            .map(|pid| PartitionInfo {
                partition_id: pid,
                owner_node_id: format!("node-{}", pid as usize % node_count),
                backup_node_ids: vec![],
            })
            .collect();

        PartitionMapPayload {
            version: 1,
            partition_count: PARTITION_COUNT,
            nodes,
            partitions,
            generated_at: 1_700_000_000_000,
        }
    }

    #[test]
    fn ac4_partition_table_from_payload() {
        let payload = make_test_payload(3);
        let table = PartitionTable::from_payload(&payload);

        assert_eq!(table.version(), 1);
        assert_eq!(table.partition_count(), PARTITION_COUNT);

        // Verify each partition has the correct owner
        for pid in 0..PARTITION_COUNT {
            let expected = format!("node-{}", pid as usize % 3);
            assert_eq!(
                table.get_owner(pid),
                Some(expected.as_str()),
                "partition {pid} owner mismatch"
            );
        }
    }

    // ---- AC-5: PartitionTable get_owner_for_key ----

    #[test]
    fn ac5_get_owner_for_key_hello() {
        let payload = make_test_payload(3);
        let table = PartitionTable::from_payload(&payload);

        // hash_to_partition("hello") = 95, owner = "node-{95 % 3}" = "node-2"
        let owner = table.get_owner_for_key("hello");
        assert_eq!(owner, Some("node-2"));

        // Verify it matches direct lookup
        assert_eq!(owner, table.get_owner(95));
    }

    // ---- AC-6: Partition pruning -- where clause ----

    #[test]
    fn ac6_pruning_where_key() {
        let mut where_clause = HashMap::new();
        where_clause.insert(
            "_key".to_string(),
            rmpv::Value::String("hello".into()),
        );
        let query = Query {
            r#where: Some(where_clause),
            ..Default::default()
        };
        let result = get_relevant_partitions(&query);
        assert_eq!(result, Some(vec![95]));
    }

    // ---- AC-7: Partition pruning -- predicate ----

    #[test]
    fn ac7_pruning_predicate_eq() {
        let query = Query {
            predicate: Some(PredicateNode {
                op: PredicateOp::Eq,
                attribute: Some("_key".to_string()),
                value: Some(rmpv::Value::String("hello".into())),
                children: None,
            }),
            ..Default::default()
        };
        let result = get_relevant_partitions(&query);
        assert_eq!(result, Some(vec![95]));
    }

    // ---- AC-8: Partition pruning -- unprunable ----

    #[test]
    fn ac8_pruning_or_returns_none() {
        let query = Query {
            predicate: Some(PredicateNode {
                op: PredicateOp::Or,
                attribute: None,
                value: None,
                children: Some(vec![
                    PredicateNode {
                        op: PredicateOp::Eq,
                        attribute: Some("_key".to_string()),
                        value: Some(rmpv::Value::String("a".into())),
                        children: None,
                    },
                    PredicateNode {
                        op: PredicateOp::Eq,
                        attribute: Some("_key".to_string()),
                        value: Some(rmpv::Value::String("b".into())),
                        children: None,
                    },
                ]),
            }),
            ..Default::default()
        };
        assert_eq!(get_relevant_partitions(&query), None);
    }

    #[test]
    fn ac8_pruning_no_where_no_predicate_returns_none() {
        let query = Query::default();
        assert_eq!(get_relevant_partitions(&query), None);
    }

    // ---- AC-9: owner_nodes_for_partitions ----

    #[test]
    fn ac9_owner_nodes_for_partitions() {
        let payload = make_test_payload(3);
        let table = PartitionTable::from_payload(&payload);

        // Partitions 0,1,2 -> owners node-0, node-1, node-2
        let mut owners = table.owner_nodes_for_partitions(&[0, 1, 2]);
        owners.sort_unstable();
        assert_eq!(owners, vec!["node-0", "node-1", "node-2"]);
    }

    #[test]
    fn ac9_owner_nodes_deduplicated() {
        let payload = make_test_payload(3);
        let table = PartitionTable::from_payload(&payload);

        // Partitions 0 and 3 both owned by node-0 (0%3=0, 3%3=0)
        let owners = table.owner_nodes_for_partitions(&[0, 3]);
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0], "node-0");
    }

    // ---- AC-11: Partition pruning -- $in where clause ----

    #[test]
    fn ac11_pruning_where_in() {
        let mut where_clause = HashMap::new();
        where_clause.insert(
            "_key".to_string(),
            rmpv::Value::Map(vec![(
                rmpv::Value::String("$in".into()),
                rmpv::Value::Array(vec![
                    rmpv::Value::String("a".into()),
                    rmpv::Value::String("b".into()),
                ]),
            )]),
        );
        let query = Query {
            r#where: Some(where_clause),
            ..Default::default()
        };
        let result = get_relevant_partitions(&query);
        // fnv1a("a") = 3826002220, 3826002220 % 271 = 101
        // fnv1a("b") = 3876335077, 3876335077 % 271 = 128
        assert_eq!(result, Some(vec![101, 128]));
    }

    // ---- AC-10: clippy clean -- verified by CI ----

    // ---- Additional tests for coverage ----

    #[test]
    fn new_table_all_unassigned() {
        let table = PartitionTable::new();
        assert_eq!(table.version(), 0);
        assert_eq!(table.partition_count(), PARTITION_COUNT);
        for pid in 0..PARTITION_COUNT {
            assert_eq!(table.get_owner(pid), None);
        }
    }

    #[test]
    fn set_owner_and_get_owner() {
        let mut table = PartitionTable::new();
        table.set_owner(42, "my-node".to_string());
        assert_eq!(table.get_owner(42), Some("my-node"));
        assert_eq!(table.get_owner(43), None);
    }

    #[test]
    #[should_panic(expected = "out of range")]
    fn set_owner_out_of_range_panics() {
        let mut table = PartitionTable::new();
        table.set_owner(PARTITION_COUNT, "bad".to_string());
    }

    #[test]
    fn partitions_for_node_returns_correct_set() {
        let payload = make_test_payload(3);
        let table = PartitionTable::from_payload(&payload);

        let node0_partitions = table.partitions_for_node("node-0");
        // node-0 owns partitions 0, 3, 6, 9, ..., i.e., all pid where pid % 3 == 0
        assert!(node0_partitions.len() > 80); // 271/3 ~ 90
        for pid in &node0_partitions {
            assert_eq!(*pid % 3, 0);
        }
    }

    #[test]
    fn partitions_for_unknown_node_is_empty() {
        let payload = make_test_payload(3);
        let table = PartitionTable::from_payload(&payload);
        assert!(table.partitions_for_node("unknown").is_empty());
    }

    #[test]
    fn pruning_where_eq_operator() {
        let mut where_clause = HashMap::new();
        where_clause.insert(
            "key".to_string(),
            rmpv::Value::Map(vec![(
                rmpv::Value::String("$eq".into()),
                rmpv::Value::String("hello".into()),
            )]),
        );
        let query = Query {
            r#where: Some(where_clause),
            ..Default::default()
        };
        assert_eq!(get_relevant_partitions(&query), Some(vec![95]));
    }

    #[test]
    fn pruning_where_integer_key() {
        let mut where_clause = HashMap::new();
        where_clause.insert(
            "id".to_string(),
            rmpv::Value::Integer(42.into()),
        );
        let query = Query {
            r#where: Some(where_clause),
            ..Default::default()
        };
        let result = get_relevant_partitions(&query);
        assert!(result.is_some());
        let pids = result.unwrap();
        assert_eq!(pids.len(), 1);
        assert_eq!(pids[0], hash_to_partition("42"));
    }

    #[test]
    fn pruning_where_array_of_keys() {
        let mut where_clause = HashMap::new();
        where_clause.insert(
            "_key".to_string(),
            rmpv::Value::Array(vec![
                rmpv::Value::String("hello".into()),
                rmpv::Value::String("key1".into()),
            ]),
        );
        let query = Query {
            r#where: Some(where_clause),
            ..Default::default()
        };
        let result = get_relevant_partitions(&query);
        // hello -> 95, key1 -> 268
        let mut expected = vec![95, 268];
        expected.sort_unstable();
        assert_eq!(result, Some(expected));
    }

    #[test]
    fn pruning_predicate_and_with_key_children() {
        let query = Query {
            predicate: Some(PredicateNode {
                op: PredicateOp::And,
                attribute: None,
                value: None,
                children: Some(vec![
                    PredicateNode {
                        op: PredicateOp::Eq,
                        attribute: Some("_key".to_string()),
                        value: Some(rmpv::Value::String("hello".into())),
                        children: None,
                    },
                    PredicateNode {
                        op: PredicateOp::Gt,
                        attribute: Some("age".to_string()),
                        value: Some(rmpv::Value::Integer(18.into())),
                        children: None,
                    },
                ]),
            }),
            ..Default::default()
        };
        // Should extract "hello" from the eq child
        assert_eq!(get_relevant_partitions(&query), Some(vec![95]));
    }

    #[test]
    fn pruning_predicate_not_returns_none() {
        let query = Query {
            predicate: Some(PredicateNode {
                op: PredicateOp::Not,
                attribute: None,
                value: None,
                children: Some(vec![PredicateNode {
                    op: PredicateOp::Eq,
                    attribute: Some("_key".to_string()),
                    value: Some(rmpv::Value::String("hello".into())),
                    children: None,
                }]),
            }),
            ..Default::default()
        };
        assert_eq!(get_relevant_partitions(&query), None);
    }

    #[test]
    fn pruning_non_key_attribute_returns_none() {
        let query = Query {
            predicate: Some(PredicateNode {
                op: PredicateOp::Eq,
                attribute: Some("name".to_string()),
                value: Some(rmpv::Value::String("hello".into())),
                children: None,
            }),
            ..Default::default()
        };
        assert_eq!(get_relevant_partitions(&query), None);
    }

    #[test]
    fn pruning_where_takes_precedence_over_predicate() {
        let mut where_clause = HashMap::new();
        where_clause.insert(
            "_key".to_string(),
            rmpv::Value::String("hello".into()),
        );
        let query = Query {
            r#where: Some(where_clause),
            predicate: Some(PredicateNode {
                op: PredicateOp::Eq,
                attribute: Some("_key".to_string()),
                value: Some(rmpv::Value::String("key1".into())),
                children: None,
            }),
            ..Default::default()
        };
        // Where clause should take precedence
        assert_eq!(get_relevant_partitions(&query), Some(vec![95]));
    }

    #[test]
    fn owner_nodes_for_unassigned_partitions() {
        let table = PartitionTable::new();
        let owners = table.owner_nodes_for_partitions(&[0, 1, 2]);
        assert!(owners.is_empty());
    }

    #[test]
    fn get_owner_out_of_range_returns_none() {
        let table = PartitionTable::new();
        assert_eq!(table.get_owner(PARTITION_COUNT), None);
        assert_eq!(table.get_owner(u32::MAX), None);
    }

    #[test]
    fn default_trait_matches_new() {
        let a = PartitionTable::new();
        let b = PartitionTable::default();
        assert_eq!(a.version(), b.version());
        assert_eq!(a.partition_count(), b.partition_count());
        for pid in 0..PARTITION_COUNT {
            assert_eq!(a.get_owner(pid), b.get_owner(pid));
        }
    }
}
