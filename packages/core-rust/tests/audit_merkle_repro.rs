// Permanent regression proof for the Merkle root-hash collision class.
//
// Run: SDKROOT=$(xcrun --sdk macosx --show-sdk-path) cargo test -p topgun-core --test audit_merkle_repro -- --nocapture
//
// The previous additive (`wrapping_add`) Merkle aggregation let two divergent
// trees share a root hash via a compensating change across entries, making the
// root-equality "are we in sync?" signal unsound -> a silent missed diff in
// delta-sync. The two `assert_ne!` tests below pin that the collision is no
// longer constructible under the order-independent, collision-resistant combine;
// the negative control proves the detection is specific to the divergence, not a
// tree bug.

use topgun_core::merkle::MerkleTree;

#[test]
fn additive_root_hash_collision_hides_divergence() {
    let mut a = MerkleTree::default_depth();
    a.update("a", 100);
    a.update("b", 200);

    let mut b = MerkleTree::default_depth();
    b.update("a", 250); // DIFFERENT content
    b.update("b", 50); // additive scheme would offset so the sum still matched

    // The collision-resistant combine must diverge at the bucket level...
    assert_ne!(
        a.get_buckets(""),
        b.get_buckets(""),
        "divergent leaves must produce divergent bucket hashes"
    );
    // ...and at the root level: two different trees must NOT share a root hash.
    assert_ne!(
        a.get_root_hash(),
        b.get_root_hash(),
        "two DIFFERENT trees must report DIFFERENT root hashes"
    );
}

#[test]
fn root_equality_used_as_in_sync_signal_is_unsound() {
    let mut local = MerkleTree::default_depth();
    local.update("k1", 0xAAAA_0000);
    local.update("k2", 0x0000_5555);

    let mut remote = MerkleTree::default_depth();
    remote.update("k1", 0xAAAA_5555); // k1 differs
    remote.update("k2", 0x0000_0000); // k2 differs; additive sum would have matched

    // Roots differ, so a root-only comparison correctly decides the replicas are
    // out of sync and drills down -- the divergent keys get reconciled.
    assert_ne!(
        local.get_root_hash(),
        remote.get_root_hash(),
        "root-only sync must detect that k1 AND k2 both diverge"
    );
    let would_sync = local.get_root_hash() != remote.get_root_hash();
    assert!(
        would_sync,
        "root-equality must NOT conclude IN-SYNC when both keys diverge"
    );
}

#[test]
fn negative_control_uncompensated_divergence_is_detected() {
    let mut a = MerkleTree::default_depth();
    a.update("a", 100);
    a.update("b", 200);

    let mut b = MerkleTree::default_depth();
    b.update("a", 250); // different, NOT compensated
    b.update("b", 200);

    assert_ne!(
        a.get_root_hash(),
        b.get_root_hash(),
        "uncompensated divergence MUST be detected"
    );
}
