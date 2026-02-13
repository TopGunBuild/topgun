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

/// Combines multiple hash values into a single order-independent hash.
///
/// Uses wrapping addition (`u32::wrapping_add`), which produces the same
/// result as the TypeScript `(result + h) | 0` followed by `>>> 0` since
/// overflow behavior is identical modulo 2^32.
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
        result = result.wrapping_add(h);
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
        let h = fnv1a_hash("single");
        assert_eq!(combine_hashes(&[h]), h);
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
        let h = fnv1a_hash("test");
        assert_eq!(combine_hashes(&[h, 0]), h);
        assert_eq!(combine_hashes(&[0, h]), h);
    }

    #[test]
    fn combine_hashes_overflow() {
        let result = combine_hashes(&[0xFFFF_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF]);
        // 3 * 0xFFFFFFFF = 0x2FFFFFFFD, mod 2^32 = 0xFFFFFFFD
        assert_eq!(result, 0xFFFF_FFFD);
    }

    #[test]
    fn combine_hashes_different_sets_different_results() {
        let s1 = combine_hashes(&[fnv1a_hash("a"), fnv1a_hash("b")]);
        let s2 = combine_hashes(&[fnv1a_hash("c"), fnv1a_hash("d")]);
        assert_ne!(s1, s2);
    }
}
