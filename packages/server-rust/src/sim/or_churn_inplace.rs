//! OR-Map churn convergence under a network-partition fault, exercising the
//! in-place mutate write path.
//!
//! The OR write path now mutates the resident `RecordValue::OrMap` in place
//! (`RecordStore::update_in_place`) instead of cloning + rebuilding + re-putting
//! the whole per-key snapshot. This simulation drives real OR_ADD / OR_REMOVE
//! ops through the full `CrdtService` op path on both sides of a network
//! partition, then heals and Merkle-syncs, and asserts the cluster converges
//! with add-wins (every concurrently-added tag survives) and remove-wins (a
//! tag removed on one side does not resurrect on the other) — the invariants
//! the in-place mutation must preserve under the partition-then-merge fault.

#[cfg(test)]
mod tests {
    use crate::sim::cluster::SimCluster;
    use crate::storage::record::RecordValue;

    /// Live entry tags in an OR-Map value.
    fn live_tags(value: &RecordValue) -> Vec<String> {
        match value {
            RecordValue::OrMap { records, .. } => {
                records.iter().map(|e| e.tag.clone()).collect()
            }
            _ => Vec::new(),
        }
    }

    /// Observed-remove tombstone tags in an OR-Map value.
    fn tombstone_tags(value: &RecordValue) -> Vec<String> {
        match value {
            RecordValue::OrMap { tombstones, .. } => tombstones.clone(),
            RecordValue::OrTombstones { tags } => tags.clone(),
            _ => Vec::new(),
        }
    }

    /// Fault scenario: two nodes churn OR adds/removes in isolation across a
    /// network partition, then heal + Merkle-sync. The in-place OR write path
    /// must converge both nodes to the same value, keeping every concurrently
    /// added tag (add-wins) and suppressing the tag removed on node 0
    /// (remove-wins — no resurrection).
    #[tokio::test]
    async fn or_churn_across_partition_converges_inplace() {
        let mut cluster = SimCluster::new(2, 4747);
        cluster.start().expect("cluster should start");

        let (map, key) = ("ormap", "doc");

        // Partition the two nodes so neither sees the other's churn.
        cluster.inject_partition(&[0], &[1]);

        // Churn: each isolated side adds its own five uniquely-tagged entries.
        for i in 0..5 {
            cluster
                .or_write(
                    0,
                    map,
                    key,
                    format!("a{i}"),
                    rmpv::Value::String(format!("va{i}").into()),
                )
                .await
                .expect("or_write on partitioned node 0 should succeed");
            cluster
                .or_write(
                    1,
                    map,
                    key,
                    format!("b{i}"),
                    rmpv::Value::String(format!("vb{i}").into()),
                )
                .await
                .expect("or_write on partitioned node 1 should succeed");
        }

        // Node 0 removes one of its own tags while still partitioned — its
        // tombstone must cross the merge and keep the tag suppressed on node 1.
        cluster
            .or_remove(0, map, key, "a2")
            .await
            .expect("or_remove on partitioned node 0 should succeed");

        // Heal and Merkle-sync both directions to carry every entry + tombstone.
        cluster.heal_partition();
        cluster
            .merkle_sync_pair(0, 1, map)
            .await
            .expect("merkle sync 0→1 after heal should succeed");
        cluster
            .merkle_sync_pair(1, 0, map)
            .await
            .expect("merkle sync 1→0 after heal should succeed");

        let converged = cluster
            .assert_converged(map, key)
            .await
            .expect("assert_converged should not error")
            .expect("both nodes should hold a value after convergence");

        let live = live_tags(&converged);
        let tombs = tombstone_tags(&converged);

        // Add-wins: every un-removed tag from BOTH partitioned sides survives.
        for i in 0..5 {
            if i != 2 {
                assert!(
                    live.contains(&format!("a{i}")),
                    "add-wins: node 0's tag a{i} must survive the merge (live={live:?})"
                );
            }
            assert!(
                live.contains(&format!("b{i}")),
                "add-wins: node 1's tag b{i} must survive the merge (live={live:?})"
            );
        }

        // Remove-wins: the tag removed on node 0 is absent from the live set and
        // recorded as a tombstone — no resurrection through the in-place merge.
        assert!(
            !live.contains(&"a2".to_string()),
            "remove-wins: removed tag a2 must not resurrect (live={live:?})"
        );
        assert!(
            tombs.contains(&"a2".to_string()),
            "remove-wins: removed tag a2 must be recorded as a tombstone (tombstones={tombs:?})"
        );
    }
}
