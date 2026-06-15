//! FNV-1a hash utilities for `MerkleTree` bucket routing.
//!
//! Provides a 32-bit FNV-1a hash that iterates over UTF-16 code units to match
//! the TypeScript `String.charCodeAt()` behavior, ensuring identical hashes
//! across Rust and TypeScript for cross-language `MerkleTree` synchronization.
//!
//! # Cross-language compatibility
//!
//! JavaScript strings are UTF-16 encoded. The TypeScript `fnv1aHash` function
//! iterates over `charCodeAt(i)`, which yields UTF-16 code units (including
//! surrogate pairs for characters outside the BMP). This Rust implementation
//! converts to UTF-16 before hashing to produce identical results.

/// FNV-1a offset basis (32-bit).
const FNV_OFFSET_BASIS: u32 = 0x811c_9dc5;

/// FNV-1a prime (32-bit).
const FNV_PRIME: u32 = 0x0100_0193;

/// Computes a 32-bit FNV-1a hash of a string, iterating over UTF-16 code units.
///
/// This matches the TypeScript `fnv1aHash()` implementation which uses
/// `String.charCodeAt()` (UTF-16) and `Math.imul()` for the multiply step.
///
/// # Examples
///
/// ```
/// use topgun_core::hash::fnv1a_hash;
///
/// assert_eq!(fnv1a_hash("hello"), 1_335_831_723);
/// assert_eq!(fnv1a_hash(""), 2_166_136_261); // FNV offset basis
/// ```
#[must_use]
pub fn fnv1a_hash(s: &str) -> u32 {
    let mut hash = FNV_OFFSET_BASIS;
    for code_unit in s.encode_utf16() {
        hash ^= u32::from(code_unit);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Avalanche finalizer constants (`MurmurHash3` `fmix32`).
const MIX_C1: u32 = 0x85eb_ca6b;
const MIX_C2: u32 = 0xc2b2_ae35;

/// Avalanche-mixes a single 32-bit hash so that small input differences spread
/// across all output bits (`MurmurHash3` `fmix32`).
///
/// This is the non-linear step that defeats compensating-pair collisions: the
/// combine sums `mix(h)`, not raw `h`, so two entry sets whose raw values happen
/// to share a sum (e.g. `100 + 200` vs `250 + 50`) no longer share a combined
/// hash, because `mix` destroys the linear relationship between inputs.
///
/// `mix(0) == 0`, which keeps zero an additive identity so the empty-set and
/// remove-all invariants still resolve to `0`.
///
/// # Cross-language note
///
/// TypeScript must reproduce this bit-for-bit with `Math.imul` for the multiply
/// steps and `>>> 0` to stay in unsigned 32-bit space:
///
/// ```ts
/// function mix(h: number): number {
///   h = (h ^ (h >>> 16)) >>> 0;
///   h = Math.imul(h, 0x85ebca6b) >>> 0;
///   h = (h ^ (h >>> 13)) >>> 0;
///   h = Math.imul(h, 0xc2b2ae35) >>> 0;
///   h = (h ^ (h >>> 16)) >>> 0;
///   return h >>> 0;
/// }
/// ```
#[must_use]
fn mix(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(MIX_C1);
    h ^= h >> 13;
    h = h.wrapping_mul(MIX_C2);
    h ^= h >> 16;
    h
}

/// Combines multiple hash values into a single order-independent, collision-
/// resistant `u32` hash.
///
/// # Algorithm
///
/// `combine([h0, h1, ...]) = (mix(h0) + mix(h1) + ...) mod 2^32`, where `mix`
/// is the `MurmurHash3` `fmix32` avalanche finalizer above and `+` is wrapping
/// 32-bit addition.
///
/// Wrapping addition over the abelian group `(Z/2^32, +)` makes the combine:
/// - **order-independent** — `combine([a, b, c]) == combine([c, a, b])` (trie
///   buckets and the server's cross-partition fold iterate in non-deterministic
///   order);
/// - **associative across calls** — because each leaf value contributes exactly
///   `mix(h)` to the sum, a plain `wrapping_add` of two `combine` outputs equals
///   the `combine` of the union of their inputs, so the server (309b) can fold
///   per-partition roots pairwise.
///
/// Summing `mix(h)` rather than raw `h` removes the compensating-pair collision
/// class: there is no easily-constructible `{a, b} != {a', b'}` with
/// `combine([a, b]) == combine([a', b'])`.
///
/// Empty input combines to `0`; a single input `[h]` combines to the stable
/// value `mix(h)` (not `h`).
///
/// # Cross-language vector (anchor for the TS port)
///
/// `combine_hashes(&[0x0000_0064, 0x0000_00c8]) == 0xbc1d_ab1c`
/// (i.e. inputs `100` and `200`). The TS `combineHashes` MUST reproduce this
/// exact `u32`; see the pinned-vector test below.
///
/// # Examples
///
/// ```
/// use topgun_core::hash::combine_hashes;
///
/// // Order-independent: same result regardless of input order
/// let a = combine_hashes(&[10, 20, 30]);
/// let b = combine_hashes(&[30, 10, 20]);
/// assert_eq!(a, b);
/// ```
#[must_use]
pub fn combine_hashes(hashes: &[u32]) -> u32 {
    let mut result: u32 = 0;
    for &h in hashes {
        result = result.wrapping_add(mix(h));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- AC-5 test vectors: exact values from spec ----

    #[test]
    fn fnv1a_hash_hello() {
        assert_eq!(fnv1a_hash("hello"), 1_335_831_723);
    }

    #[test]
    fn fnv1a_hash_key1() {
        assert_eq!(fnv1a_hash("key1"), 927_623_783);
    }

    #[test]
    fn fnv1a_hash_empty() {
        assert_eq!(fnv1a_hash(""), 2_166_136_261); // 0x811c9dc5
    }

    #[test]
    fn fnv1a_hash_key1_timestamp() {
        assert_eq!(fnv1a_hash("key1:100:0:test"), 3_988_528_110);
    }

    // ---- Basic properties ----

    #[test]
    fn fnv1a_hash_returns_nonzero_for_nonempty() {
        let hash = fnv1a_hash("test");
        assert_ne!(hash, 0);
    }

    #[test]
    fn fnv1a_hash_different_strings_different_hashes() {
        assert_ne!(fnv1a_hash("hello"), fnv1a_hash("world"));
    }

    #[test]
    fn fnv1a_hash_deterministic() {
        let h1 = fnv1a_hash("consistent-string");
        let h2 = fnv1a_hash("consistent-string");
        let h3 = fnv1a_hash("consistent-string");
        assert_eq!(h1, h2);
        assert_eq!(h2, h3);
    }

    #[test]
    fn fnv1a_hash_case_sensitive() {
        let h1 = fnv1a_hash("Hello");
        let h2 = fnv1a_hash("hello");
        let h3 = fnv1a_hash("HELLO");
        assert_ne!(h1, h2);
        assert_ne!(h2, h3);
        assert_ne!(h1, h3);
    }

    #[test]
    fn fnv1a_hash_single_char() {
        let hash = fnv1a_hash("a");
        assert!(hash > 0);
    }

    #[test]
    fn fnv1a_hash_long_string() {
        let long = "a".repeat(10_000);
        let hash = fnv1a_hash(&long);
        assert!(hash > 0);
    }

    #[test]
    fn fnv1a_hash_whitespace_matters() {
        let h1 = fnv1a_hash("hello world");
        let h2 = fnv1a_hash("hello  world");
        let h3 = fnv1a_hash(" hello world");
        assert_ne!(h1, h2);
        assert_ne!(h1, h3);
    }

    #[test]
    fn fnv1a_hash_distribution_1000_unique() {
        let mut set = std::collections::HashSet::new();
        for i in 0..1000 {
            set.insert(fnv1a_hash(&format!("item-{i}")));
        }
        assert_eq!(set.len(), 1000);
    }

    // ---- combine_hashes tests ----

    #[test]
    fn combine_hashes_empty() {
        assert_eq!(combine_hashes(&[]), 0);
    }

    #[test]
    fn combine_hashes_single() {
        // A single input combines to the stable avalanche-mixed value of `h`
        // (no longer `h` itself), and the result is deterministic.
        let h = fnv1a_hash("single");
        assert_eq!(combine_hashes(&[h]), mix(h));
        assert_eq!(combine_hashes(&[h]), combine_hashes(&[h]));
    }

    #[test]
    fn combine_hashes_order_independent() {
        let h1 = fnv1a_hash("first");
        let h2 = fnv1a_hash("second");
        let h3 = fnv1a_hash("third");

        let c1 = combine_hashes(&[h1, h2, h3]);
        let c2 = combine_hashes(&[h3, h1, h2]);
        let c3 = combine_hashes(&[h2, h3, h1]);

        assert_eq!(c1, c2);
        assert_eq!(c2, c3);
    }

    #[test]
    fn combine_hashes_with_zero() {
        // `mix(0) == 0`, so zero remains an additive identity: appending a
        // zero-valued entry does not change the combined hash.
        let h = fnv1a_hash("test");
        assert_eq!(combine_hashes(&[h, 0]), combine_hashes(&[h]));
        assert_eq!(combine_hashes(&[0, h]), combine_hashes(&[h]));
        assert_eq!(mix(0), 0);
    }

    #[test]
    fn combine_hashes_overflow() {
        // The fold wraps modulo 2^32 over the mixed values.
        let result = combine_hashes(&[0xFFFF_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF]);
        let expected = mix(0xFFFF_FFFF)
            .wrapping_mul(3)
            .wrapping_add(0)
            .wrapping_add(0);
        assert_eq!(result, expected);
        assert_eq!(result, 0x85d4_4dab);
    }

    #[test]
    fn combine_hashes_different_sets_different_results() {
        let s1 = combine_hashes(&[fnv1a_hash("a"), fnv1a_hash("b")]);
        let s2 = combine_hashes(&[fnv1a_hash("c"), fnv1a_hash("d")]);
        assert_ne!(s1, s2);
    }

    /// Pinned cross-language test vector. SPEC-309c's `hash.test.ts` MUST
    /// reproduce this exact `u32` output bit-for-bit; it is the anchor that
    /// proves the Rust and TS `combineHashes` implementations agree.
    #[test]
    fn combine_hashes_cross_language_pinned_vector() {
        assert_eq!(combine_hashes(&[0x0000_0064, 0x0000_00c8]), 0xbc1d_ab1c);
    }

    #[test]
    fn combine_hashes_resists_compensating_pairs() {
        // The two collision vectors from the audit reproduction: under a plain
        // additive fold `100 + 200 == 250 + 50` and
        // `0xAAAA0000 + 0x5555 == 0xAAAA5555 + 0`, so divergent sets collided.
        // The avalanche-mixed combine must keep them distinct.
        assert_ne!(combine_hashes(&[100, 200]), combine_hashes(&[250, 50]));
        assert_ne!(
            combine_hashes(&[0xAAAA_0000, 0x0000_5555]),
            combine_hashes(&[0xAAAA_5555, 0x0000_0000])
        );
    }

    #[test]
    fn combine_hashes_associative_across_calls() {
        // Folding two `combine` outputs with `wrapping_add` equals combining the
        // union — the property the server's cross-partition fold relies on.
        let a = [fnv1a_hash("a"), fnv1a_hash("b")];
        let b = [fnv1a_hash("c"), fnv1a_hash("d")];
        let union: Vec<u32> = a.iter().chain(b.iter()).copied().collect();
        assert_eq!(
            combine_hashes(&a).wrapping_add(combine_hashes(&b)),
            combine_hashes(&union)
        );
    }
}
