//! Partition assignment and rebalancing algorithms.
//!
//! Three free functions for deterministic partition distribution:
//! - `compute_assignment()`: modulo-based partition ownership with round-robin backups
//! - `plan_rebalance()`: diff current state against target to produce migration tasks
//! - `order_migrations()`: sort migrations for availability preservation

use super::state::ClusterPartitionTable;
use super::types::{MemberInfo, MigrationTask, NodeState, PartitionAssignment};

/// Computes a deterministic partition assignment for the given active members.
///
/// Uses modulo-based distribution with lexicographic node ordering for
/// reproducibility. Backup replicas are assigned round-robin from the next
/// `backup_count` members after the owner (wrapping around).
///
/// Returns an empty Vec if no active members exist.
#[must_use]
pub fn compute_assignment(
    members: &[MemberInfo],
    partition_count: u32,
    backup_count: u32,
) -> Vec<PartitionAssignment> {
    // Filter to Active members only and sort by node_id for determinism.
    let mut active: Vec<&MemberInfo> = members
        .iter()
        .filter(|m| m.state == NodeState::Active)
        .collect();
    active.sort_by(|a, b| a.node_id.cmp(&b.node_id));

    let n = active.len();
    if n == 0 {
        return Vec::new();
    }

    let mut assignments = Vec::with_capacity(partition_count as usize);

    for pid in 0..partition_count {
        let owner_idx = (pid as usize) % n;
        let owner = active[owner_idx].node_id.clone();

        // Assign backups from the next members in the sorted list (wrapping).
        // Skip if only 1 active member -- backup must be on a different node.
        let mut backups = Vec::new();
        if n > 1 {
            let actual_backup_count = (backup_count as usize).min(n - 1);
            for i in 1..=actual_backup_count {
                let backup_idx = (owner_idx + i) % n;
                backups.push(active[backup_idx].node_id.clone());
            }
        }

        assignments.push(PartitionAssignment {
            partition_id: pid,
            owner,
            backups,
        });
    }

    assignments
}

/// Plans migration tasks to transition from current partition state to target.
///
/// Compares each target assignment against the current partition table. If the
/// owner differs, a `MigrationTask` is created. Partitions with no current
/// entry in the table are skipped -- unassigned partitions should be populated
/// via `apply_assignments()` directly, not through migration (there is no
/// source node to migrate from).
///
/// Returns migrations sorted by `partition_id` for deterministic ordering.
#[must_use]
pub fn plan_rebalance(
    current: &ClusterPartitionTable,
    target: &[PartitionAssignment],
) -> Vec<MigrationTask> {
    let mut tasks = Vec::new();

    for assignment in target {
        // Skip partitions with no current entry -- no source to migrate from.
        let Some(current_meta) = current.get_partition(assignment.partition_id) else {
            continue;
        };

        // Only create a migration if the owner is changing.
        if current_meta.owner != assignment.owner {
            tasks.push(MigrationTask {
                partition_id: assignment.partition_id,
                source: current_meta.owner.clone(),
                destination: assignment.owner.clone(),
                new_backups: assignment.backups.clone(),
            });
        }
    }

    // Sort by partition_id for deterministic ordering.
    tasks.sort_by_key(|t| t.partition_id);
    tasks
}

/// Orders migration tasks for availability preservation.
///
/// Sorts migrations to minimize data loss risk:
/// 1. Backup promotions first (destination is already a backup of the partition)
/// 2. Partitions with fewer total replicas migrate first (most at risk)
pub fn order_migrations(tasks: &mut [MigrationTask], partition_table: &ClusterPartitionTable) {
    tasks.sort_by(|a, b| {
        let a_is_promotion = is_backup_promotion(a, partition_table);
        let b_is_promotion = is_backup_promotion(b, partition_table);

        // Backup promotions first.
        match (a_is_promotion, b_is_promotion) {
            (true, false) => return std::cmp::Ordering::Less,
            (false, true) => return std::cmp::Ordering::Greater,
            _ => {}
        }

        // Fewer replicas migrate first (most at risk of data loss).
        let a_replicas = replica_count(a.partition_id, partition_table);
        let b_replicas = replica_count(b.partition_id, partition_table);

        a_replicas
            .cmp(&b_replicas)
            .then_with(|| a.partition_id.cmp(&b.partition_id))
    });
}

/// Returns `true` if the migration destination is already a backup for the partition.
fn is_backup_promotion(task: &MigrationTask, table: &ClusterPartitionTable) -> bool {
    table
        .get_partition(task.partition_id)
        .is_some_and(|meta| meta.backups.contains(&task.destination))
}

/// Returns the total replica count (owner + backups) for a partition.
fn replica_count(partition_id: u32, table: &ClusterPartitionTable) -> usize {
    table
        .get_partition(partition_id)
        .map_or(0, |meta| 1 + meta.backups.len())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::types::NodeState;

    fn make_active_member(node_id: &str) -> MemberInfo {
        MemberInfo {
            node_id: node_id.to_string(),
            host: "127.0.0.1".to_string(),
            client_port: 8080,
            cluster_port: 9090,
            state: NodeState::Active,
            join_version: 1,
        }
    }

    fn make_member_with_state(node_id: &str, state: NodeState) -> MemberInfo {
        MemberInfo {
            node_id: node_id.to_string(),
            host: "127.0.0.1".to_string(),
            client_port: 8080,
            cluster_port: 9090,
            state,
            join_version: 1,
        }
    }

    // -- compute_assignment --

    #[test]
    fn compute_empty_when_no_active_members() {
        let members = vec![make_member_with_state("node-1", NodeState::Joining)];
        let result = compute_assignment(&members, 271, 1);
        assert!(result.is_empty());
    }

    #[test]
    fn compute_empty_when_no_members() {
        let result = compute_assignment(&[], 271, 1);
        assert!(result.is_empty());
    }

    #[test]
    fn compute_single_member_no_backups() {
        let members = vec![make_active_member("node-1")];
        let result = compute_assignment(&members, 271, 1);

        assert_eq!(result.len(), 271);
        for a in &result {
            assert_eq!(a.owner, "node-1");
            // Only 1 active member -- no backups possible.
            assert!(a.backups.is_empty());
        }
    }

    #[test]
    fn compute_three_members_even_distribution() {
        let members = vec![
            make_active_member("node-a"),
            make_active_member("node-b"),
            make_active_member("node-c"),
        ];
        let result = compute_assignment(&members, 271, 1);

        assert_eq!(result.len(), 271);

        // Count assignments per node.
        let mut counts = std::collections::HashMap::new();
        for a in &result {
            *counts.entry(a.owner.clone()).or_insert(0u32) += 1;
        }

        // 271 / 3 = 90.33... => two nodes get 90, one gets 91.
        for count in counts.values() {
            assert!(
                *count == 90 || *count == 91,
                "expected ~90 partitions per node, got {count}"
            );
        }
    }

    #[test]
    fn compute_no_partition_unassigned() {
        let members = vec![
            make_active_member("node-a"),
            make_active_member("node-b"),
            make_active_member("node-c"),
        ];
        let result = compute_assignment(&members, 271, 1);

        // Every partition from 0..271 should be present.
        for pid in 0..271u32 {
            assert!(
                result.iter().any(|a| a.partition_id == pid),
                "partition {pid} is unassigned"
            );
        }
    }

    #[test]
    fn compute_backup_on_different_node() {
        let members = vec![
            make_active_member("node-a"),
            make_active_member("node-b"),
            make_active_member("node-c"),
        ];
        let result = compute_assignment(&members, 271, 1);

        for a in &result {
            assert_eq!(a.backups.len(), 1);
            assert_ne!(
                a.owner, a.backups[0],
                "backup must be on a different node than owner"
            );
        }
    }

    #[test]
    fn compute_deterministic() {
        let members = vec![
            make_active_member("node-c"),
            make_active_member("node-a"),
            make_active_member("node-b"),
        ];

        let result1 = compute_assignment(&members, 271, 1);
        let result2 = compute_assignment(&members, 271, 1);

        assert_eq!(result1, result2, "assignment must be deterministic");
    }

    #[test]
    fn compute_filters_non_active_members() {
        let members = vec![
            make_active_member("node-a"),
            make_member_with_state("node-b", NodeState::Leaving),
            make_active_member("node-c"),
        ];
        let result = compute_assignment(&members, 10, 1);

        // Only node-a and node-c should be assigned.
        for a in &result {
            assert!(
                a.owner == "node-a" || a.owner == "node-c",
                "non-active node-b should not be assigned, got owner: {}",
                a.owner
            );
        }
    }

    #[test]
    fn compute_multiple_backups() {
        let members = vec![
            make_active_member("node-a"),
            make_active_member("node-b"),
            make_active_member("node-c"),
            make_active_member("node-d"),
        ];
        let result = compute_assignment(&members, 10, 2);

        for a in &result {
            assert_eq!(a.backups.len(), 2);
            // All backups must be different from owner and from each other.
            assert_ne!(a.owner, a.backups[0]);
            assert_ne!(a.owner, a.backups[1]);
            assert_ne!(a.backups[0], a.backups[1]);
        }
    }

    // -- plan_rebalance --

    #[test]
    fn rebalance_empty_when_current_matches_target() {
        let table = ClusterPartitionTable::new(271);
        table.set_owner(0, "node-1".to_string(), vec!["node-2".to_string()]);
        table.set_owner(1, "node-2".to_string(), vec!["node-1".to_string()]);

        let target = vec![
            PartitionAssignment {
                partition_id: 0,
                owner: "node-1".to_string(),
                backups: vec!["node-2".to_string()],
            },
            PartitionAssignment {
                partition_id: 1,
                owner: "node-2".to_string(),
                backups: vec!["node-1".to_string()],
            },
        ];

        let tasks = plan_rebalance(&table, &target);
        assert!(tasks.is_empty());
    }

    #[test]
    fn rebalance_detects_owner_change() {
        let table = ClusterPartitionTable::new(271);
        table.set_owner(0, "node-1".to_string(), vec![]);
        table.set_owner(1, "node-2".to_string(), vec![]);

        let target = vec![
            PartitionAssignment {
                partition_id: 0,
                owner: "node-2".to_string(),
                backups: vec!["node-1".to_string()],
            },
            PartitionAssignment {
                partition_id: 1,
                owner: "node-2".to_string(),
                backups: vec!["node-1".to_string()],
            },
        ];

        let tasks = plan_rebalance(&table, &target);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].partition_id, 0);
        assert_eq!(tasks[0].source, "node-1");
        assert_eq!(tasks[0].destination, "node-2");
    }

    #[test]
    fn rebalance_skips_unassigned_partitions() {
        let table = ClusterPartitionTable::new(271);
        // Partition 0 exists, partition 1 does not.
        table.set_owner(0, "node-1".to_string(), vec![]);

        let target = vec![
            PartitionAssignment {
                partition_id: 0,
                owner: "node-2".to_string(),
                backups: vec![],
            },
            PartitionAssignment {
                partition_id: 1,
                owner: "node-2".to_string(),
                backups: vec![],
            },
        ];

        let tasks = plan_rebalance(&table, &target);
        // Only partition 0 should produce a migration (partition 1 has no current entry).
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].partition_id, 0);
    }

    #[test]
    fn rebalance_sorted_by_partition_id() {
        let table = ClusterPartitionTable::new(271);
        table.set_owner(5, "node-1".to_string(), vec![]);
        table.set_owner(2, "node-1".to_string(), vec![]);
        table.set_owner(8, "node-1".to_string(), vec![]);

        let target = vec![
            PartitionAssignment {
                partition_id: 8,
                owner: "node-2".to_string(),
                backups: vec![],
            },
            PartitionAssignment {
                partition_id: 2,
                owner: "node-2".to_string(),
                backups: vec![],
            },
            PartitionAssignment {
                partition_id: 5,
                owner: "node-2".to_string(),
                backups: vec![],
            },
        ];

        let tasks = plan_rebalance(&table, &target);
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].partition_id, 2);
        assert_eq!(tasks[1].partition_id, 5);
        assert_eq!(tasks[2].partition_id, 8);
    }

    // -- order_migrations --

    #[test]
    fn order_backup_promotions_first() {
        let table = ClusterPartitionTable::new(271);
        // Partition 0: owner=node-1, backup=node-2 (migrating to node-2 = promotion)
        table.set_owner(0, "node-1".to_string(), vec!["node-2".to_string()]);
        // Partition 1: owner=node-1, backup=node-3 (migrating to node-4 = NOT promotion)
        table.set_owner(1, "node-1".to_string(), vec!["node-3".to_string()]);

        let mut tasks = vec![
            MigrationTask {
                partition_id: 1,
                source: "node-1".to_string(),
                destination: "node-4".to_string(),
                new_backups: vec![],
            },
            MigrationTask {
                partition_id: 0,
                source: "node-1".to_string(),
                destination: "node-2".to_string(),
                new_backups: vec![],
            },
        ];

        order_migrations(&mut tasks, &table);

        // Backup promotion (partition 0) should come first.
        assert_eq!(tasks[0].partition_id, 0);
        assert_eq!(tasks[1].partition_id, 1);
    }

    #[test]
    fn order_fewer_replicas_first() {
        let table = ClusterPartitionTable::new(271);
        // Partition 0: owner + 2 backups = 3 replicas
        table.set_owner(
            0,
            "node-1".to_string(),
            vec!["node-2".to_string(), "node-3".to_string()],
        );
        // Partition 1: owner + 0 backups = 1 replica (most at risk)
        table.set_owner(1, "node-1".to_string(), vec![]);

        let mut tasks = vec![
            MigrationTask {
                partition_id: 0,
                source: "node-1".to_string(),
                destination: "node-4".to_string(),
                new_backups: vec![],
            },
            MigrationTask {
                partition_id: 1,
                source: "node-1".to_string(),
                destination: "node-4".to_string(),
                new_backups: vec![],
            },
        ];

        order_migrations(&mut tasks, &table);

        // Partition 1 (fewer replicas) should come first.
        assert_eq!(tasks[0].partition_id, 1);
        assert_eq!(tasks[1].partition_id, 0);
    }
}
