//! OpenAI-compatible HTTP embedding provider.
//!
//! Sends requests to `/v1/embeddings` with `{"model": ..., "input": [...]}` body
//! and parses `{"data": [{"embedding": [...]}]}` response. Supports optional Bearer auth.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::{EmbeddingError, EmbeddingProvider, HttpProviderConfig};

/// Embedding provider backed by any OpenAI-compatible /v1/embeddings endpoint.
pub struct HttpEmbeddingProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    model: String,
    dimension: u16,
}

impl HttpEmbeddingProvider {
    /// # Panics
    ///
    /// Panics if the `reqwest::Client` cannot be constructed (e.g., invalid TLS config).
    /// In practice this never happens with the default builder.
    #[must_use]
    pub fn new(config: HttpProviderConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build reqwest client for HTTP embedding provider");
        Self {
            client,
            base_url: config.base_url,
            api_key: config.api_key,
            model: config.model,
            dimension: config.dimension,
        }
    }
}

#[derive(Serialize)]
struct OpenAiRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct OpenAiResponse {
    data: Vec<OpenAiEmbedding>,
}

#[derive(Deserialize)]
struct OpenAiEmbedding {
    embedding: Vec<f32>,
}

#[async_trait]
impl EmbeddingProvider for HttpEmbeddingProvider {
    fn name(&self) -> &'static str {
        "http"
    }

    fn dimension(&self) -> u16 {
        self.dimension
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let texts = vec![text.to_string()];
        let mut results = self.batch_embed(&texts).await?;
        results
            .pop()
            .ok_or_else(|| EmbeddingError::InvalidResponse("empty data array".to_string()))
    }

    async fn batch_embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let url = format!("{}/v1/embeddings", self.base_url);
        let body = OpenAiRequest {
            model: &self.model,
            input: texts,
        };

        let mut request = self.client.post(&url).json(&body);

        if let Some(ref key) = self.api_key {
            request = request.bearer_auth(key);
        }

        let response = request
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
                "embedding API returned HTTP {status}: {text}"
            )));
        }

        let parsed: OpenAiResponse = response
            .json()
            .await
            .map_err(|e| EmbeddingError::InvalidResponse(e.to_string()))?;

        let mut embeddings = Vec::with_capacity(parsed.data.len());
        for item in parsed.data {
            let actual = u16::try_from(item.embedding.len()).unwrap_or(u16::MAX);
            if actual != self.dimension {
                return Err(EmbeddingError::DimensionMismatch {
                    expected: self.dimension,
                    actual,
                });
            }
            embeddings.push(item.embedding);
        }

        Ok(embeddings)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::domain::embedding::HttpProviderConfig;

    #[test]
    fn test_http_provider_name() {
        let provider = HttpEmbeddingProvider::new(HttpProviderConfig {
            base_url: "http://api.example.com".to_string(),
            api_key: None,
            model: "text-embedding-3-small".to_string(),
            dimension: 1536,
        });
        assert_eq!(provider.name(), "http");
    }

    #[test]
    fn test_http_provider_dimension() {
        let provider = HttpEmbeddingProvider::new(HttpProviderConfig {
            base_url: "http://api.example.com".to_string(),
            api_key: None,
            model: "text-embedding-3-small".to_string(),
            dimension: 1536,
        });
        assert_eq!(provider.dimension(), 1536);
    }

    #[test]
    fn test_http_provider_with_api_key() {
        let provider = HttpEmbeddingProvider::new(HttpProviderConfig {
            base_url: "http://api.openai.com".to_string(),
            api_key: Some("sk-test-key".to_string()),
            model: "text-embedding-ada-002".to_string(),
            dimension: 1536,
        });
        assert_eq!(provider.name(), "http");
        assert_eq!(provider.api_key, Some("sk-test-key".to_string()));
    }

    #[test]
    fn test_http_provider_without_api_key() {
        let provider = HttpEmbeddingProvider::new(HttpProviderConfig {
            base_url: "http://localhost:8080".to_string(),
            api_key: None,
            model: "local-model".to_string(),
            dimension: 512,
        });
        assert!(provider.api_key.is_none());
    }
}
