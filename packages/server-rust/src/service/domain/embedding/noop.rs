//! Noop embedding provider — returns zero vectors of the configured dimension.
//!
//! Used for testing, development, and situations where embedding generation
//! is not required but the interface must be satisfied.

use async_trait::async_trait;

use super::{EmbeddingError, EmbeddingProvider, NoopConfig};

/// Embedding provider that returns zero vectors without making any HTTP calls.
pub struct NoopEmbeddingProvider {
    dimension: u16,
}

impl NoopEmbeddingProvider {
    pub fn new(config: NoopConfig) -> Self {
        Self {
            dimension: config.dimension,
        }
    }
}

#[async_trait]
impl EmbeddingProvider for NoopEmbeddingProvider {
    fn name(&self) -> &str {
        "noop"
    }

    fn dimension(&self) -> u16 {
        self.dimension
    }

    async fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbeddingError> {
        Ok(vec![0.0f32; self.dimension as usize])
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_noop_embed_returns_zero_vector() {
        let provider = NoopEmbeddingProvider::new(NoopConfig { dimension: 4 });
        let result = provider.embed("hello world").await.unwrap();
        assert_eq!(result.len(), 4);
        assert!(result.iter().all(|&v| v == 0.0f32));
    }

    #[tokio::test]
    async fn test_noop_embed_correct_dimension() {
        let provider = NoopEmbeddingProvider::new(NoopConfig { dimension: 768 });
        let result = provider.embed("test").await.unwrap();
        assert_eq!(result.len(), 768);
    }

    #[test]
    fn test_noop_provider_name() {
        let provider = NoopEmbeddingProvider::new(NoopConfig { dimension: 4 });
        assert_eq!(provider.name(), "noop");
    }

    #[test]
    fn test_noop_provider_dimension() {
        let provider = NoopEmbeddingProvider::new(NoopConfig { dimension: 128 });
        assert_eq!(provider.dimension(), 128);
    }

    #[tokio::test]
    async fn test_noop_batch_embed_uses_default_sequential() {
        let provider = NoopEmbeddingProvider::new(NoopConfig { dimension: 4 });
        let texts = vec!["hello".to_string(), "world".to_string()];
        let results = provider.batch_embed(&texts).await.unwrap();
        assert_eq!(results.len(), 2);
        for vec in &results {
            assert_eq!(vec.len(), 4);
            assert!(vec.iter().all(|&v| v == 0.0f32));
        }
    }
}
