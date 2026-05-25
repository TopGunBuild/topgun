//! Integration tests for parallel 3-node cluster bootstrap and master election.
//!
//! These tests construct `ClusterFormationService` instances directly against
//! real `TcpListener` sockets on loopback (`127.0.0.1:0`) and drive
//! `discover_seeds_and_join` concurrently. No madsim wiring is required; tests
//! use the real tokio runtime with real TCP connections.
//!
//! The tests verify:
//! - Exactly one master emerges from a parallel 3-node cold-start (no staggering).
//! - The elected master is always the lexicographically lowest `node_id`.
//! - All three nodes converge to the same membership view within the election budget.
//! - Startup jitter (50–500ms random delays) does not change the elected master.
//! - The proptest tiebreak property holds across 100 seeded RNG iterations.

use std::sync::Arc;
use std::time::Duration;

use rand::rngs::StdRng;
use rand::Rng as _;
use rand::SeedableRng;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use topgun_server::cluster::formation::ClusterFormationService;
use topgun_server::cluster::peer_connection::PeerConnectionMap;
use topgun_server::cluster::state::{ClusterState, InboundClusterMessage};
use topgun_server::cluster::{ClusterConfig, MemberInfo, NodeState};

// ---------------------------------------------------------------------------
// Test constants
// ---------------------------------------------------------------------------

/// Total wall-clock budget for convergence assertions:
/// `MASTER_ELECTION_TOTAL_BUDGET_MS` (30s) + 5s slack for CI CPU contention.
const CONVERGENCE_BUDGET_MS: u64 = 35_000;

/// Number of proptest tiebreak iterations. All 4 tests in this file are annotated
/// with `#[serial_test::serial]`, which forces cargo to schedule them one at a time
/// regardless of `--test-threads` setting. Serial execution eliminates the port and
/// CPU contention that would otherwise starve follower nodes under default parallelism,
/// making 100 iterations safe and matching AC #5.
const PROPTEST_ITERATIONS: u32 = 100;

// ---------------------------------------------------------------------------
// Test harness helpers
// ---------------------------------------------------------------------------

/// Bind a TCP listener on an OS-assigned loopback port and return the listener + port.
async fn bind_listener() -> (TcpListener, u16) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback listener");
    let port = listener.local_addr().expect("local_addr").port();
    (listener, port)
}

/// Start all nodes concurrently and wait up to `budget_ms` for all to converge
/// to `expected_node_count` Active members in their membership views.
async fn wait_for_convergence(nodes: &[Arc<ClusterFormationService>], budget_ms: u64) {
    let deadline = std::time::Instant::now() + Duration::from_millis(budget_ms);
    loop {
        if std::time::Instant::now() >= deadline {
            // Print diagnostics before panicking
            for (i, node) in nodes.iter().enumerate() {
                let view = node.cluster_state.current_view();
                eprintln!(
                    "Node {i}: is_master={}, members={}, view_version={}",
                    node.cluster_state.is_master(),
                    view.members.len(),
                    view.version,
                );
            }
            panic!("Cluster did not converge within {budget_ms}ms");
        }

        let all_ready = nodes.iter().all(|n| {
            let view = n.cluster_state.current_view();
            view.members
                .iter()
                .filter(|m| m.state == NodeState::Active)
                .count()
                == nodes.len()
        });

        if all_ready {
            return;
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// ---------------------------------------------------------------------------
// Test: parallel_boot_elects_single_master
// ---------------------------------------------------------------------------

/// Verifies that 3 nodes spawned in parallel (no staggering, full peer seed lists)
/// converge to exactly one master — the node with the lexicographically lowest
/// `node_id` — within `CONVERGENCE_BUDGET_MS` on loopback TCP.
#[tokio::test(flavor = "multi_thread")]
#[serial_test::serial]
async fn parallel_boot_elects_single_master() {
    // Step 1: bind all cluster listeners to get OS-assigned ports
    let (l0, p0) = bind_listener().await;
    let (l1, p1) = bind_listener().await;
    let (l2, p2) = bind_listener().await;

    // Step 2: construct nodes with full peer seed lists (excluding self)
    let seeds0 = vec![format!("127.0.0.1:{p1}"), format!("127.0.0.1:{p2}")];
    let seeds1 = vec![format!("127.0.0.1:{p0}"), format!("127.0.0.1:{p2}")];
    let seeds2 = vec![format!("127.0.0.1:{p0}"), format!("127.0.0.1:{p1}")];

    let config0 = Arc::new(ClusterConfig {
        cluster_id: "test-cluster".to_string(),
        seed_addresses: seeds0,
        ..ClusterConfig::default()
    });
    let config1 = Arc::new(ClusterConfig {
        cluster_id: "test-cluster".to_string(),
        seed_addresses: seeds1,
        ..ClusterConfig::default()
    });
    let config2 = Arc::new(ClusterConfig {
        cluster_id: "test-cluster".to_string(),
        seed_addresses: seeds2,
        ..ClusterConfig::default()
    });

    let mk_member = |node_id: &str, cluster_port: u16| MemberInfo {
        node_id: node_id.to_string(),
        host: "127.0.0.1".to_string(),
        client_port: 0,
        cluster_port,
        state: NodeState::Joining,
        join_version: 0,
    };

    let mk_node = |config: Arc<ClusterConfig>, node_id: &str, cluster_port: u16| {
        let (cs, _rx) = ClusterState::new(Arc::clone(&config), node_id.to_string());
        let cs = Arc::new(cs);
        let peers = Arc::new(PeerConnectionMap::new());
        let (tx, _rx) = mpsc::unbounded_channel::<InboundClusterMessage>();
        Arc::new(ClusterFormationService::new(
            cs,
            peers,
            config,
            mk_member(node_id, cluster_port),
            tx,
        ))
    };

    let svc0 = mk_node(config0, "node-0", p0);
    let svc1 = mk_node(config1, "node-1", p1);
    let svc2 = mk_node(config2, "node-2", p2);

    // Step 3: start all three concurrently (no staggering)
    Arc::clone(&svc0).start(l0);
    Arc::clone(&svc1).start(l1);
    Arc::clone(&svc2).start(l2);

    // Step 4: wait for convergence
    let nodes: Vec<Arc<ClusterFormationService>> =
        vec![Arc::clone(&svc0), Arc::clone(&svc1), Arc::clone(&svc2)];
    wait_for_convergence(&nodes, CONVERGENCE_BUDGET_MS).await;

    // Step 5: assert exactly one master, correct identity, consistent views
    let master_count = nodes.iter().filter(|n| n.cluster_state.is_master()).count();
    assert_eq!(
        master_count, 1,
        "Exactly one node should be master; got {master_count}"
    );

    // The master must be node-0 (lexicographically lowest of node-0, node-1, node-2)
    assert!(
        svc0.cluster_state.is_master(),
        "node-0 (lowest node_id) must be the elected master"
    );
    assert!(!svc1.cluster_state.is_master(), "node-1 must not be master");
    assert!(!svc2.cluster_state.is_master(), "node-2 must not be master");

    // All views must report 3 active members
    for (i, node) in nodes.iter().enumerate() {
        let view = node.cluster_state.current_view();
        let active = view
            .members
            .iter()
            .filter(|m| m.state == NodeState::Active)
            .count();
        assert_eq!(
            active, 3,
            "Node {i} should see 3 active members; saw {active}"
        );
    }

    // All views must agree on master node_id via the MembersView::master() logic
    for (i, node) in nodes.iter().enumerate() {
        let view = node.cluster_state.current_view();
        let master_in_view = view.master().map(|m| m.node_id.as_str());
        // The master by view logic (lowest join_version, then node_id) should be node-0
        // since node-0 self-promotes first (lowest-id tiebreak) with join_version=1
        assert!(
            master_in_view == Some("node-0"),
            "Node {i} view reports master={master_in_view:?}, expected Some(\"node-0\")"
        );
    }
}

// ---------------------------------------------------------------------------
// Test: parallel_boot_with_loopback_delays
// ---------------------------------------------------------------------------

/// Same as `parallel_boot_elects_single_master` but with random startup jitter
/// (50–500ms per node) to simulate cold-build CPU-contention startup skew.
/// The lowest-id node (`node-0`) must still win.
#[tokio::test(flavor = "multi_thread")]
#[serial_test::serial]
async fn parallel_boot_with_loopback_delays() {
    let (l0, p0) = bind_listener().await;
    let (l1, p1) = bind_listener().await;
    let (l2, p2) = bind_listener().await;

    let seeds_for = |self_port: u16, all_ports: &[u16]| {
        all_ports
            .iter()
            .filter(|&&p| p != self_port)
            .map(|p| format!("127.0.0.1:{p}"))
            .collect::<Vec<_>>()
    };

    let all_ports = [p0, p1, p2];

    let mk_config = |self_port: u16| {
        Arc::new(ClusterConfig {
            cluster_id: "test-cluster".to_string(),
            seed_addresses: seeds_for(self_port, &all_ports),
            ..ClusterConfig::default()
        })
    };

    let mk_member = |node_id: &str, cluster_port: u16| MemberInfo {
        node_id: node_id.to_string(),
        host: "127.0.0.1".to_string(),
        client_port: 0,
        cluster_port,
        state: NodeState::Joining,
        join_version: 0,
    };

    let mk_node = |config: Arc<ClusterConfig>, node_id: &str, cluster_port: u16| {
        let (cs, _rx) = ClusterState::new(Arc::clone(&config), node_id.to_string());
        let cs = Arc::new(cs);
        let peers = Arc::new(PeerConnectionMap::new());
        let (tx, _rx) = mpsc::unbounded_channel::<InboundClusterMessage>();
        Arc::new(ClusterFormationService::new(
            cs,
            peers,
            config,
            mk_member(node_id, cluster_port),
            tx,
        ))
    };

    let svc0 = mk_node(mk_config(p0), "node-0", p0);
    let svc1 = mk_node(mk_config(p1), "node-1", p1);
    let svc2 = mk_node(mk_config(p2), "node-2", p2);

    // Seeded RNG for reproducible jitter values
    let mut rng = StdRng::seed_from_u64(42);
    let jitter0: u64 = rng.random_range(50..=500);
    let jitter1: u64 = rng.random_range(50..=500);
    let jitter2: u64 = rng.random_range(50..=500);

    // Start nodes concurrently with random jitter
    let svc0_c = Arc::clone(&svc0);
    let svc1_c = Arc::clone(&svc1);
    let svc2_c = Arc::clone(&svc2);

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(jitter0)).await;
        svc0_c.start(l0);
    });
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(jitter1)).await;
        svc1_c.start(l1);
    });
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(jitter2)).await;
        svc2_c.start(l2);
    });

    let nodes: Vec<Arc<ClusterFormationService>> =
        vec![Arc::clone(&svc0), Arc::clone(&svc1), Arc::clone(&svc2)];

    // Allow extra time for the jitter + election budget
    wait_for_convergence(&nodes, CONVERGENCE_BUDGET_MS + 500).await;

    let master_count = nodes.iter().filter(|n| n.cluster_state.is_master()).count();
    assert_eq!(master_count, 1, "Exactly one master; got {master_count}");
    assert!(svc0.cluster_state.is_master(), "node-0 must be master");
}

// ---------------------------------------------------------------------------
// Test: deterministic_tiebreak_property
// ---------------------------------------------------------------------------

/// Proptest-style: across 100 seeded RNG iterations, the node with the
/// lexicographically lowest `node_id` always wins the election.
///
/// Uses `block_in_place` + `Handle::block_on` per CLAUDE.md Proptest Async
/// Bridge pattern; `multi_thread` flavor is required because `block_in_place`
/// panics on a single-threaded runtime.
#[tokio::test(flavor = "multi_thread")]
#[serial_test::serial]
#[allow(clippy::too_many_lines)]
async fn deterministic_tiebreak_property() {
    use proptest::prelude::*;
    use proptest::test_runner::{Config as PropConfig, TestRunner};
    use tokio::runtime::Handle;

    let handle = Handle::current();

    // Generate 3 distinct node_id strings using independent strategies.
    // RegexGeneratorStrategy does not implement Clone, so we build 3 separately.
    let strategy = (
        proptest::string::string_regex("[a-z]{4,8}-[0-9]").unwrap(),
        proptest::string::string_regex("[a-z]{4,8}-[0-9]").unwrap(),
        proptest::string::string_regex("[a-z]{4,8}-[0-9]").unwrap(),
    );

    let config = PropConfig {
        cases: PROPTEST_ITERATIONS,
        ..PropConfig::default()
    };

    let mut runner = TestRunner::new(config);

    let result = runner.run(&strategy, |(id_a, id_b, id_c): (String, String, String)| {
        tokio::task::block_in_place(|| {
            handle.block_on(async {
                // Find the expected minimum
                let mut ids = [id_a.clone(), id_b.clone(), id_c.clone()];
                ids.sort();
                let expected_master = ids[0].clone();

                // Bind listeners
                let (la, pa) = bind_listener().await;
                let (lb, pb) = bind_listener().await;
                let (lc, pc) = bind_listener().await;

                let mk_config_for = |others: &[u16]| {
                    Arc::new(ClusterConfig {
                        cluster_id: "prop-cluster".to_string(),
                        seed_addresses: others.iter().map(|p| format!("127.0.0.1:{p}")).collect(),
                        ..ClusterConfig::default()
                    })
                };

                let mk_member = |nid: &str, cp: u16| MemberInfo {
                    node_id: nid.to_string(),
                    host: "127.0.0.1".to_string(),
                    client_port: 0,
                    cluster_port: cp,
                    state: NodeState::Joining,
                    join_version: 0,
                };

                let mk_node = |config: Arc<ClusterConfig>, nid: &str, cp: u16| {
                    let (cs, _rx) = ClusterState::new(Arc::clone(&config), nid.to_string());
                    let cs = Arc::new(cs);
                    let peers = Arc::new(PeerConnectionMap::new());
                    let (tx, _rx2) = mpsc::unbounded_channel::<InboundClusterMessage>();
                    Arc::new(ClusterFormationService::new(
                        cs,
                        peers,
                        config,
                        mk_member(nid, cp),
                        tx,
                    ))
                };

                let ca = mk_config_for(&[pb, pc]);
                let cb = mk_config_for(&[pa, pc]);
                let cc = mk_config_for(&[pa, pb]);

                let sva = mk_node(ca, &id_a, pa);
                let svb = mk_node(cb, &id_b, pb);
                let svc = mk_node(cc, &id_c, pc);

                Arc::clone(&sva).start(la);
                Arc::clone(&svb).start(lb);
                Arc::clone(&svc).start(lc);

                let nodes = [Arc::clone(&sva), Arc::clone(&svb), Arc::clone(&svc)];

                // Wait for convergence (shorter budget per iteration to keep total time bounded)
                let iter_budget_ms = 12_000u64;
                let deadline = std::time::Instant::now() + Duration::from_millis(iter_budget_ms);
                loop {
                    if std::time::Instant::now() >= deadline {
                        break;
                    }
                    let all_converged = nodes.iter().all(|n| {
                        let view = n.cluster_state.current_view();
                        view.members
                            .iter()
                            .filter(|m| m.state == NodeState::Active)
                            .count()
                            == 3
                    });
                    if all_converged {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }

                // Tiebreak assertion: lowest-id must be master
                let master_count = nodes.iter().filter(|n| n.cluster_state.is_master()).count();

                // Find the actual master
                let actual_master: Option<&str> = [(&sva, &id_a), (&svb, &id_b), (&svc, &id_c)]
                    .iter()
                    .find(|(svc, _)| svc.cluster_state.is_master())
                    .map(|(_, id)| id.as_str());

                prop_assert_eq!(
                    master_count,
                    1,
                    "Expected exactly 1 master, got {} for ids ({}, {}, {})",
                    master_count,
                    id_a,
                    id_b,
                    id_c
                );

                prop_assert_eq!(
                    actual_master,
                    Some(expected_master.as_str()),
                    "Expected master={} (lowest id), got {:?} for ids ({}, {}, {})",
                    expected_master,
                    actual_master,
                    id_a,
                    id_b,
                    id_c
                );

                Ok(())
            })
        })
    });

    if let Err(e) = result {
        panic!("deterministic_tiebreak_property failed: {e}");
    }
}

// ---------------------------------------------------------------------------
// Test: permanent_rejection_does_not_trigger_election
// ---------------------------------------------------------------------------

/// A node configured with a wrong `cluster_id` should receive `PermanentRejection`
/// from the master and NOT enter `WaitForMasterElection`. It should time out
/// its total budget and self-promote as a single-node master (isolated cluster).
///
/// Verifies AC #9: permanent rejections short-circuit to the next seed without
/// triggering election wait.
#[tokio::test(flavor = "multi_thread")]
#[serial_test::serial]
async fn permanent_rejection_does_not_trigger_election() {
    // Bootstrap a normal 2-node cluster first
    let (l0, p0) = bind_listener().await;
    let (l1, p1) = bind_listener().await;

    let mk_node = |nid: &str, port: u16, seeds: Vec<String>, cluster_id: &str| {
        let config = Arc::new(ClusterConfig {
            cluster_id: cluster_id.to_string(),
            seed_addresses: seeds,
            ..ClusterConfig::default()
        });
        let local = MemberInfo {
            node_id: nid.to_string(),
            host: "127.0.0.1".to_string(),
            client_port: 0,
            cluster_port: port,
            state: NodeState::Joining,
            join_version: 0,
        };
        let (cs, _rx) = ClusterState::new(Arc::clone(&config), nid.to_string());
        let cs = Arc::new(cs);
        let peers = Arc::new(PeerConnectionMap::new());
        let (tx, _rx2) = mpsc::unbounded_channel::<InboundClusterMessage>();
        Arc::new(ClusterFormationService::new(cs, peers, config, local, tx))
    };

    // Node-0: correct cluster_id, no seeds (self-promotes immediately)
    let svc0 = mk_node("node-0", p0, vec![], "cluster-A");
    Arc::clone(&svc0).start(l0);

    // Wait for node-0 to self-promote
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "node-0 did not self-promote within 5s"
        );
        if svc0.cluster_state.is_master() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Node-1: wrong cluster_id — join should be permanently rejected
    // (WrongClusterId reason), node-1 should NOT wait for MasterElected and
    // should eventually self-promote as its own isolated single-node cluster.
    let svc1 = mk_node(
        "node-1",
        p1,
        vec![format!("127.0.0.1:{p0}")],
        "cluster-B", // wrong cluster_id
    );
    Arc::clone(&svc1).start(l1);

    // Give node-1 time to contact node-0, get permanently rejected, exhaust
    // its seed list (no WaitForMasterElection since PermanentRejection), and
    // eventually fall through to safety-valve self-promote.
    // The total budget is 30s but with a single seed and permanent rejection,
    // the code breaks immediately after PermanentRejection (no held streams,
    // all_retryable=false), so self-promote happens almost instantly.
    let budget = Duration::from_secs(10);
    let deadline = std::time::Instant::now() + budget;
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "node-1 (wrong cluster_id) did not self-promote within {budget:?}"
        );
        if svc1.cluster_state.is_master() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Both nodes are master of their own isolated clusters — no merge.
    assert!(
        svc0.cluster_state.is_master(),
        "node-0 should still be master of cluster-A"
    );
    assert!(
        svc1.cluster_state.is_master(),
        "node-1 should be master of its own isolated cluster-B"
    );

    // Cluster sizes are independent: node-0 sees 1 member, node-1 sees 1 member.
    let view0 = svc0.cluster_state.current_view();
    let view1 = svc1.cluster_state.current_view();
    assert_eq!(
        view0
            .members
            .iter()
            .filter(|m| m.state == NodeState::Active)
            .count(),
        1,
        "node-0 should see only itself"
    );
    assert_eq!(
        view1
            .members
            .iter()
            .filter(|m| m.state == NodeState::Active)
            .count(),
        1,
        "node-1 should see only itself (isolated cluster)"
    );
}
