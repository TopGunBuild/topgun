//! Ollama embedding provider using the /api/embed endpoint (Ollama 0.5+).
//!
//! Sends batch embedding requests to the Ollama HTTP API with
//! `{"model": ..., "input": [...]}` body and parses `{"embeddings": [[...]]}` response.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::{EmbeddingError, EmbeddingProvider, OllamaConfig};

/// Embedding provider backed by the Ollama HTTP API.
pub struct OllamaEmbeddingProvider {
    client: reqwest::Client,
    base_url: String,
    model: String,
    dimension: u16,
}

impl OllamaEmbeddingProvider {
    /// # Panics
    ///
    /// Panics if the `reqwest::Client` cannot be constructed (e.g., invalid TLS config).
    /// In practice this never happens with the default builder.
    #[must_use]
    pub fn new(config: OllamaConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build reqwest client for Ollama provider");
        Self {
            client,
            base_url: config.base_url,
            model: config.model,
            dimension: config.dimension,
        }
    }
}

#[derive(Serialize)]
struct OllamaRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct OllamaResponse {
    embeddings: Vec<Vec<f32>>,
}

#[async_trait]
impl EmbeddingProvider for OllamaEmbeddingProvider {
    fn name(&self) -> &'static str {
        "ollama"
    }

    fn dimension(&self) -> u16 {
        self.dimension
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let texts = vec![text.to_string()];
        let mut results = self.batch_embed(&texts).await?;
        results
            .pop()
            .ok_or_else(|| EmbeddingError::InvalidResponse("empty embeddings array".to_string()))
    }

    async fn batch_embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let url = format!("{}/api/embed", self.base_url);
        let body = OllamaRequest {
            model: &self.model,
            input: texts,
        };

        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| EmbeddingError::Http(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable>".to_string());
            return Err(EmbeddingError::Http(format!(
                "Ollama returned HTTP {status}: {text}"
            )));
        }

        let parsed: OllamaResponse = response
            .json()
            .await
            .map_err(|e| EmbeddingError::InvalidResponse(e.to_string()))?;

        // Validate dimension of each returned vector.
        for vec in &parsed.embeddings {
            let actual = u16::try_from(vec.len()).unwrap_or(u16::MAX);
            if actual != self.dimension {
                return Err(EmbeddingError::DimensionMismatch {
                    expected: self.dimension,
                    actual,
                });
            }
        }

        Ok(parsed.embeddings)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::domain::embedding::OllamaConfig;

    #[test]
    fn test_ollama_provider_name() {
        let provider = OllamaEmbeddingProvider::new(OllamaConfig {
            base_url: "http://localhost:11434".to_string(),
            model: "nomic-embed-text".to_string(),
            dimension: 768,
        });
        assert_eq!(provider.name(), "ollama");
    }

    #[test]
    fn test_ollama_provider_dimension() {
        let provider = OllamaEmbeddingProvider::new(OllamaConfig {
            base_url: "http://localhost:11434".to_string(),
            model: "nomic-embed-text".to_string(),
            dimension: 768,
        });
        assert_eq!(provider.dimension(), 768);
    }
}
