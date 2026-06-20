//! Deterministic embedding provider — hashes text into a stable vector.
//!
//! Unlike [`NoopEmbeddingProvider`](super::noop::NoopEmbeddingProvider), which
//! returns all-zero vectors that carry no semantic signal, this provider produces
//! a reproducible bag-of-words vector: tokens are hashed into dimension buckets,
//! term frequencies are accumulated, and the result is L2-normalized.
//!
//! Properties that make it suitable for CI integration tests of the semantic
//! search path, without requiring an external embedding service:
//! - **Deterministic**: the same text always maps to the same vector, on every
//!   run and platform (FNV-1a hashing, no RNG, no global hasher state).
//! - **Semantically ordered**: texts that share tokens have higher cosine
//!   similarity than texts that share none, so nearest-neighbour ranking is
//!   meaningful enough to assert on.
//!
//! This is NOT a real embedding model — it has no understanding of synonyms or
//! word order. It exists so the vector/hybrid pipeline can be exercised
//! end-to-end deterministically. Production deployments use the Ollama or HTTP
//! providers.

use async_trait::async_trait;

use super::{EmbeddingError, EmbeddingProvider};

/// Embedding provider that derives a stable hashed bag-of-words vector from text.
pub struct DeterministicEmbeddingProvider {
    dimension: u16,
}

impl DeterministicEmbeddingProvider {
    #[must_use]
    pub fn new(dimension: u16) -> Self {
        Self { dimension }
    }

    /// Splits text into lowercase alphanumeric tokens.
    fn tokenize(text: &str) -> impl Iterator<Item = String> + '_ {
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .map(str::to_lowercase)
    }

    /// FNV-1a 64-bit hash — deterministic across runs and platforms, no seed state.
    fn fnv1a(token: &str) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in token.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }
}

#[async_trait]
impl EmbeddingProvider for DeterministicEmbeddingProvider {
    fn name(&self) -> &'static str {
        "deterministic"
    }

    fn dimension(&self) -> u16 {
        self.dimension
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let dim = self.dimension as usize;
        let mut vec = vec![0.0f32; dim];

        for token in Self::tokenize(text) {
            let h = Self::fnv1a(&token);
            // Bucket the term, and use a second hash bit to assign a sign so that
            // distinct tokens colliding into the same bucket do not always add
            // constructively — this spreads the signal across the space.
            // `h % dim` is always < dim (<= u16::MAX), so it fits usize on every target.
            let bucket = usize::try_from(h % dim as u64).unwrap_or(0);
            let sign = if (h >> 63) & 1 == 0 { 1.0 } else { -1.0 };
            vec[bucket] += sign;
        }

        // L2-normalize so cosine similarity is well-defined. Empty / token-less
        // text yields a zero vector (the write-path hook skips empty text, and a
        // zero query vector simply matches nothing rather than erroring).
        let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in &mut vec {
                *v /= norm;
            }
        }

        Ok(vec)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
    }

    #[tokio::test]
    async fn same_text_same_vector() {
        let p = DeterministicEmbeddingProvider::new(64);
        let a = p.embed("wireless bluetooth headphones").await.unwrap();
        let b = p.embed("wireless bluetooth headphones").await.unwrap();
        assert_eq!(a, b, "deterministic provider must be reproducible");
        assert_eq!(a.len(), 64);
    }

    #[tokio::test]
    async fn normalized_unit_length() {
        let p = DeterministicEmbeddingProvider::new(128);
        let v = p.embed("some non empty text here").await.unwrap();
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "expected unit norm, got {norm}");
    }

    #[tokio::test]
    async fn shared_tokens_rank_above_disjoint() {
        // A query is more similar to a doc sharing tokens than to one sharing none.
        let p = DeterministicEmbeddingProvider::new(256);
        let query = p.embed("wireless bluetooth headphones").await.unwrap();
        let related = p
            .embed("noise cancelling wireless headphones over ear")
            .await
            .unwrap();
        let unrelated = p.embed("fresh organic banana fruit basket").await.unwrap();

        let sim_related = cosine(&query, &related);
        let sim_unrelated = cosine(&query, &unrelated);
        assert!(
            sim_related > sim_unrelated,
            "shared-token doc ({sim_related}) should outrank disjoint doc ({sim_unrelated})"
        );
    }

    #[tokio::test]
    async fn empty_text_is_zero_vector() {
        let p = DeterministicEmbeddingProvider::new(16);
        let v = p.embed("").await.unwrap();
        assert!(v.iter().all(|&x| x == 0.0));
    }
}
