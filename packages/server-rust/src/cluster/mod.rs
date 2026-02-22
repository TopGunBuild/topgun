//! Cluster protocol module.
//!
//! Provides domain types, service traits, wire messages, shared state,
//! failure detection, and partition assignment algorithms for the
//! inter-node cluster protocol.

pub mod assignment;
pub mod failure_detector;
pub mod messages;
pub mod state;
pub mod traits;
pub mod types;

// ---------------------------------------------------------------------------
// Re-exports — flat public API
// ---------------------------------------------------------------------------

// types
pub use types::{
    ActiveMigration, ClusterConfig, ClusterHealth, MemberInfo, MembersView, MigrationPhase,
    MigrationTask, NodeState, PartitionAssignment, PartitionMeta, PartitionState,
};

// traits
pub use traits::{
    ClusterPartitionService, ClusterService, FailureDetector, MembershipService, MigrationService,
};

// messages
pub use messages::{
    ClusterMessage, DeltaOp, ExplicitSuspicionPayload, HeartbeatComplaintPayload,
    HeartbeatPayload, JoinRequestPayload, JoinResponsePayload, LeaveRequestPayload, MapStateChunk,
    MapType, MembersUpdatePayload, MigrateCancelPayload, MigrateDataPayload,
    MigrateFinalizePayload, MigrateReadyPayload, MigrateStartPayload, MergeRequestPayload,
    OpForwardPayload, PartitionTableUpdatePayload, SplitBrainProbePayload,
    SplitBrainProbeResponsePayload,
};

// state
pub use state::{
    ClusterChange, ClusterChannelReceivers, ClusterChannels, ClusterPartitionTable, ClusterState,
    InboundClusterMessage, MigrationCommand,
};

// failure_detector
pub use failure_detector::{DeadlineFailureDetector, PhiAccrualConfig, PhiAccrualFailureDetector};

// assignment
pub use assignment::{compute_assignment, order_migrations, plan_rebalance};

// ---------------------------------------------------------------------------
// Integration tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod integration_tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Test Category 1: Serde Round-Trip Tests
    // -----------------------------------------------------------------------

    /// Helper: serialize to MsgPack named and deserialize back, asserting equality.
    fn round_trip(msg: &ClusterMessage) {
        let bytes = rmp_serde::to_vec_named(msg).expect("serialize failed");
        let decoded: ClusterMessage =
            rmp_serde::from_slice(&bytes).expect("deserialize failed");
        assert_eq!(msg, &decoded);
    }

    fn sample_members_view() -> MembersView {
        MembersView {
            version: 3,
            members: vec![
                MemberInfo {
                    node_id: "node-1".to_string(),
                    host: "10.0.0.1".to_string(),
                    client_port: 8080,
                    cluster_port: 9090,
                    state: NodeState::Active,
                    join_version: 1,
                },
                MemberInfo {
                    node_id: "node-2".to_string(),
                    host: "10.0.0.2".to_string(),
                    client_port: 8080,
                    cluster_port: 9090,
                    state: NodeState::Joining,
                    join_version: 2,
                },
            ],
        }
    }

    #[test]
    fn serde_join_request_with_auth_token() {
        round_trip(&ClusterMessage::JoinRequest(JoinRequestPayload {
            node_id: "node-3".to_string(),
            host: "10.0.0.3".to_string(),
            client_port: 8080,
            cluster_port: 9090,
            cluster_id: "cluster-1".to_string(),
            protocol_version: 1,
            auth_token: Some("secret-token".to_string()),
        }));
    }

    #[test]
    fn serde_join_response_accepted() {
        let view = sample_members_view();
        round_trip(&ClusterMessage::JoinResponse(JoinResponsePayload {
            accepted: true,
            reject_reason: None,
            members_view: Some(view),
            partition_assignments: Some(vec![PartitionAssignment {
                partition_id: 0,
                owner: "node-1".to_string(),
                backups: vec!["node-2".to_string()],
            }]),
        }));
    }

    #[test]
    fn serde_join_response_rejected() {
        round_trip(&ClusterMessage::JoinResponse(JoinResponsePayload {
            accepted: false,
            reject_reason: Some("cluster full".to_string()),
            members_view: None,
            partition_assignments: None,
        }));
    }

    #[test]
    fn serde_members_update() {
        round_trip(&ClusterMessage::MembersUpdate(MembersUpdatePayload {
            view: sample_members_view(),
            cluster_time_ms: 1_700_000_000_000,
        }));
    }

    #[test]
    fn serde_leave_request_with_reason() {
        round_trip(&ClusterMessage::LeaveRequest(LeaveRequestPayload {
            node_id: "node-2".to_string(),
            reason: Some("graceful shutdown".to_string()),
        }));
    }

    #[test]
    fn serde_leave_request_without_reason() {
        round_trip(&ClusterMessage::LeaveRequest(LeaveRequestPayload {
            node_id: "node-2".to_string(),
            reason: None,
        }));
    }

    #[test]
    fn serde_heartbeat_with_suspected_nodes() {
        round_trip(&ClusterMessage::Heartbeat(HeartbeatPayload {
            sender_id: "node-1".to_string(),
            timestamp_ms: 1_700_000_000_000,
            members_view_version: 5,
            suspected_nodes: vec!["node-3".to_string(), "node-4".to_string()],
        }));
    }

    #[test]
    fn serde_heartbeat_complaint() {
        round_trip(&ClusterMessage::HeartbeatComplaint(
            HeartbeatComplaintPayload {
                complainer_id: "node-1".to_string(),
                complainer_view_version: 5,
                suspect_id: "node-3".to_string(),
                suspect_view_version: 4,
            },
        ));
    }

    #[test]
    fn serde_explicit_suspicion() {
        round_trip(&ClusterMessage::ExplicitSuspicion(
            ExplicitSuspicionPayload {
                suspect_id: "node-3".to_string(),
                reason: "missed 5 heartbeats".to_string(),
                master_view_version: 7,
            },
        ));
    }

    #[test]
    fn serde_partition_table_update() {
        round_trip(&ClusterMessage::PartitionTableUpdate(
            PartitionTableUpdatePayload {
                assignments: vec![
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
                ],
                version: 42,
                completed_migrations: vec!["mig-001".to_string(), "mig-002".to_string()],
            },
        ));
    }

    #[test]
    fn serde_fetch_partition_table() {
        round_trip(&ClusterMessage::FetchPartitionTable);
    }

    #[test]
    fn serde_migrate_start() {
        round_trip(&ClusterMessage::MigrateStart(MigrateStartPayload {
            migration_id: "mig-100".to_string(),
            partition_id: 42,
            destination_node_id: "node-2".to_string(),
        }));
    }

    #[test]
    fn serde_migrate_data() {
        round_trip(&ClusterMessage::MigrateData(MigrateDataPayload {
            partition_id: 42,
            map_states: vec![MapStateChunk {
                map_name: "users".to_string(),
                data: vec![0xDE, 0xAD, 0xBE, 0xEF],
                map_type: MapType::Lww,
            }],
            delta_ops: vec![DeltaOp {
                map_name: "users".to_string(),
                key: "alice".to_string(),
                entry: vec![0xCA, 0xFE],
            }],
            source_version: 7,
        }));
    }

    #[test]
    fn serde_migrate_ready() {
        round_trip(&ClusterMessage::MigrateReady(MigrateReadyPayload {
            migration_id: "mig-100".to_string(),
            partition_id: 42,
            source_node_id: "node-1".to_string(),
        }));
    }

    #[test]
    fn serde_migrate_finalize() {
        round_trip(&ClusterMessage::MigrateFinalize(MigrateFinalizePayload {
            migration_id: "mig-100".to_string(),
            partition_id: 42,
            new_owner: "node-2".to_string(),
        }));
    }

    #[test]
    fn serde_migrate_cancel() {
        round_trip(&ClusterMessage::MigrateCancel(MigrateCancelPayload {
            migration_id: "mig-100".to_string(),
            partition_id: 42,
            reason: "source node left".to_string(),
        }));
    }

    #[test]
    fn serde_split_brain_probe() {
        round_trip(&ClusterMessage::SplitBrainProbe(SplitBrainProbePayload {
            sender_cluster_id: "cluster-1".to_string(),
            sender_master_id: "node-1".to_string(),
            sender_member_count: 3,
            sender_view_version: 10,
        }));
    }

    #[test]
    fn serde_split_brain_probe_response() {
        round_trip(&ClusterMessage::SplitBrainProbeResponse(
            SplitBrainProbeResponsePayload {
                responder_cluster_id: "cluster-1".to_string(),
                responder_master_id: "node-2".to_string(),
                responder_member_count: 2,
                responder_view_version: 8,
                responder_master_join_version: 1,
            },
        ));
    }

    #[test]
    fn serde_merge_request() {
        round_trip(&ClusterMessage::MergeRequest(MergeRequestPayload {
            source_cluster_id: "cluster-2".to_string(),
            source_members: vec![MemberInfo {
                node_id: "node-5".to_string(),
                host: "10.0.0.5".to_string(),
                client_port: 8080,
                cluster_port: 9090,
                state: NodeState::Active,
                join_version: 1,
            }],
            source_view_version: 3,
        }));
    }

    #[test]
    fn serde_op_forward() {
        round_trip(&ClusterMessage::OpForward(OpForwardPayload {
            source_node_id: "node-1".to_string(),
            target_partition_id: 42,
            client_id: Some("client-abc".to_string()),
            payload: vec![0x01, 0x02, 0x03, 0x04],
        }));
    }

    // -----------------------------------------------------------------------
    // Test Category 2: Re-Export Accessibility
    // -----------------------------------------------------------------------

    #[test]
    fn reexports_types_accessible() {
        // Construct values using only the cluster:: prefix (no submodule path).
        let _state = NodeState::Active;
        let _pstate = PartitionState::Unassigned;
        let _phase = MigrationPhase::Replicating;
        let _member = MemberInfo {
            node_id: "n1".to_string(),
            host: "127.0.0.1".to_string(),
            client_port: 8080,
            cluster_port: 9090,
            state: NodeState::Active,
            join_version: 1,
        };
        let _view = MembersView {
            version: 1,
            members: vec![],
        };
        let _meta = PartitionMeta {
            partition_id: 0,
            owner: "n1".to_string(),
            backups: vec![],
            state: PartitionState::Active,
            version: 0,
        };
        let _assignment = PartitionAssignment {
            partition_id: 0,
            owner: "n1".to_string(),
            backups: vec![],
        };
        let _task = MigrationTask {
            partition_id: 0,
            source: "n1".to_string(),
            destination: "n2".to_string(),
            new_backups: vec![],
        };
        let _active = ActiveMigration {
            migration_id: "m1".to_string(),
            partition_id: 0,
            source: "n1".to_string(),
            destination: "n2".to_string(),
            state: MigrationPhase::Replicating,
            started_at_ms: 0,
        };
        let _health = ClusterHealth::default();
        let _config = ClusterConfig::default();
    }

    #[test]
    fn reexports_messages_accessible() {
        let _msg = ClusterMessage::FetchPartitionTable;
        let _map_type = MapType::Lww;
        let _chunk = MapStateChunk {
            map_name: "m".to_string(),
            data: vec![],
            map_type: MapType::Or,
        };
        let _delta = DeltaOp {
            map_name: "m".to_string(),
            key: "k".to_string(),
            entry: vec![],
        };
        let _join_req = JoinRequestPayload {
            node_id: "n".to_string(),
            host: "h".to_string(),
            client_port: 0,
            cluster_port: 0,
            cluster_id: "c".to_string(),
            protocol_version: 1,
            auth_token: None,
        };
        let _join_resp = JoinResponsePayload::default();
        let _members_update = MembersUpdatePayload {
            view: MembersView {
                version: 0,
                members: vec![],
            },
            cluster_time_ms: 0,
        };
        let _leave = LeaveRequestPayload {
            node_id: "n".to_string(),
            reason: None,
        };
        let _hb = HeartbeatPayload {
            sender_id: "n".to_string(),
            timestamp_ms: 0,
            members_view_version: 0,
            suspected_nodes: vec![],
        };
        let _complaint = HeartbeatComplaintPayload {
            complainer_id: "n".to_string(),
            complainer_view_version: 0,
            suspect_id: "s".to_string(),
            suspect_view_version: 0,
        };
        let _suspicion = ExplicitSuspicionPayload {
            suspect_id: "s".to_string(),
            reason: "r".to_string(),
            master_view_version: 0,
        };
        let _ptu = PartitionTableUpdatePayload {
            assignments: vec![],
            version: 0,
            completed_migrations: vec![],
        };
        let _ms = MigrateStartPayload {
            migration_id: "m".to_string(),
            partition_id: 0,
            destination_node_id: "n".to_string(),
        };
        let _md = MigrateDataPayload {
            partition_id: 0,
            map_states: vec![],
            delta_ops: vec![],
            source_version: 0,
        };
        let _mr = MigrateReadyPayload {
            migration_id: "m".to_string(),
            partition_id: 0,
            source_node_id: "n".to_string(),
        };
        let _mf = MigrateFinalizePayload {
            migration_id: "m".to_string(),
            partition_id: 0,
            new_owner: "n".to_string(),
        };
        let _mc = MigrateCancelPayload {
            migration_id: "m".to_string(),
            partition_id: 0,
            reason: "r".to_string(),
        };
        let _sbp = SplitBrainProbePayload {
            sender_cluster_id: "c".to_string(),
            sender_master_id: "m".to_string(),
            sender_member_count: 0,
            sender_view_version: 0,
        };
        let _sbpr = SplitBrainProbeResponsePayload {
            responder_cluster_id: "c".to_string(),
            responder_master_id: "m".to_string(),
            responder_member_count: 0,
            responder_view_version: 0,
            responder_master_join_version: 0,
        };
        let _merge = MergeRequestPayload {
            source_cluster_id: "c".to_string(),
            source_members: vec![],
            source_view_version: 0,
        };
        let _fwd = OpForwardPayload {
            source_node_id: "n".to_string(),
            target_partition_id: 0,
            client_id: None,
            payload: vec![],
        };
    }

    #[test]
    fn reexports_state_accessible() {
        let _table = ClusterPartitionTable::new(271);
        let _change = ClusterChange::PartitionTableUpdated { version: 1 };
        let _cmd = MigrationCommand::CancelAll;
        let _inbound = InboundClusterMessage {
            sender_node_id: "n1".to_string(),
            message: ClusterMessage::FetchPartitionTable,
        };
        let (_channels, _receivers) = ClusterChannels::new(16);

        // ClusterState requires an Arc<ClusterConfig> and returns a receiver
        let config = std::sync::Arc::new(ClusterConfig::default());
        let (_state, _rx) = ClusterState::new(config, "node-1".to_string());
    }

    #[test]
    fn reexports_failure_detector_accessible() {
        let _config = PhiAccrualConfig::default();
        let _phi = PhiAccrualFailureDetector::new(PhiAccrualConfig::default());
        let _deadline = DeadlineFailureDetector::new(5000);

        // Verify they implement the FailureDetector trait via the re-exported trait.
        fn assert_fd(_: &dyn FailureDetector) {}
        assert_fd(&_phi);
        assert_fd(&_deadline);
    }

    #[test]
    fn reexports_assignment_accessible() {
        let members = vec![MemberInfo {
            node_id: "n1".to_string(),
            host: "127.0.0.1".to_string(),
            client_port: 8080,
            cluster_port: 9090,
            state: NodeState::Active,
            join_version: 1,
        }];

        let result = compute_assignment(&members, 10, 0);
        assert_eq!(result.len(), 10);

        let table = ClusterPartitionTable::new(10);
        let tasks = plan_rebalance(&table, &result);
        assert!(tasks.is_empty());

        let mut migrations = vec![];
        order_migrations(&mut migrations, &table);
        assert!(migrations.is_empty());
    }

    #[test]
    fn reexports_traits_accessible() {
        // Verify all 5 traits are accessible as trait objects.
        // We only need to confirm they can be named, not instantiated.
        fn _assert_cluster_service<T: ClusterService>() {}
        fn _assert_membership_service<T: MembershipService>() {}
        fn _assert_partition_service<T: ClusterPartitionService>() {}
        fn _assert_migration_service<T: MigrationService>() {}
        fn _assert_failure_detector<T: FailureDetector>() {}
    }
}
