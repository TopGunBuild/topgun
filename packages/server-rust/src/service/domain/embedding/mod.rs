//! Embedding provider abstraction and configuration.
//!
//! Defines the `EmbeddingProvider` trait, error types, and configuration structs
//! for pluggable server-side text embedding. Three concrete providers are available:
//! Ollama, OpenAI-compatible HTTP, and Noop (returns zero vectors).

use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;

pub mod http;
pub mod noop;
pub mod ollama;

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Async trait for text embedding providers. Implementors must be Send + Sync + 'static
/// to allow use behind `Arc<dyn EmbeddingProvider>`.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync + 'static {
    /// Human-readable provider name (e.g., "ollama", "http", "noop").
    fn name(&self) -> &'static str;

    /// Output vector dimensionality.
    fn dimension(&self) -> u16;

    /// Embed a single text string into a vector.
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError>;

    /// Embed multiple texts. Default implementation calls `embed` sequentially.
    async fn batch_embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let mut results = Vec::with_capacity(texts.len());
        for text in texts {
            results.push(self.embed(text).await?);
        }
        Ok(results)
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum EmbeddingError {
    #[error("HTTP request failed: {0}")]
    Http(String),
    #[error("provider returned invalid response: {0}")]
    InvalidResponse(String),
    #[error("dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: u16, actual: u16 },
    #[error("provider unavailable: {0}")]
    Unavailable(String),
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorConfig {
    pub provider: EmbeddingProviderConfig,
    pub maps: HashMap<String, MapVectorConfig>,
}

impl VectorConfig {
    /// Validate that every map's declared dimension matches the provider's dimension.
    ///
    /// # Errors
    ///
    /// Returns `Err` with a descriptive message if any `MapVectorConfig.dimension`
    /// does not equal the provider's declared dimension.
    pub fn validate(&self) -> Result<(), String> {
        let provider_dim = match &self.provider {
            EmbeddingProviderConfig::Ollama(c) => c.dimension,
            EmbeddingProviderConfig::Http(c) => c.dimension,
            EmbeddingProviderConfig::Noop(c) => c.dimension,
        };
        for (map_name, map_cfg) in &self.maps {
            if map_cfg.dimension != provider_dim {
                return Err(format!(
                    "map '{}' dimension {} does not match provider dimension {}",
                    map_name, map_cfg.dimension, provider_dim
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum EmbeddingProviderConfig {
    Ollama(OllamaConfig),
    Http(HttpProviderConfig),
    Noop(NoopConfig),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaConfig {
    #[serde(default = "default_ollama_base_url")]
    pub base_url: String,
    #[serde(default = "default_ollama_model")]
    pub model: String,
    pub dimension: u16,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpProviderConfig {
    pub base_url: String,
    #[serde(default)]
    pub api_key: Option<String>,
    pub model: String,
    pub dimension: u16,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoopConfig {
    pub dimension: u16,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapVectorConfig {
    pub fields: Vec<String>,
    #[serde(default)]
    pub index_name: Option<String>,
    /// Must equal the provider's declared dimension. Validated via `VectorConfig::validate()`.
    pub dimension: u16,
}

// ---------------------------------------------------------------------------
// Defaults
// ---------------------------------------------------------------------------

fn default_ollama_base_url() -> String {
    "http://localhost:11434".to_string()
}

fn default_ollama_model() -> String {
    "nomic-embed-text".to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::single_char_pattern)]
mod tests {
    use super::*;

    #[test]
    fn test_ollama_config_defaults() {
        let json = r#"{"dimension": 768}"#;
        let cfg: OllamaConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.base_url, "http://localhost:11434");
        assert_eq!(cfg.model, "nomic-embed-text");
        assert_eq!(cfg.dimension, 768);
    }

    #[test]
    fn test_embedding_provider_config_ollama_tag() {
        let json = r#"{"type": "ollama", "dimension": 768}"#;
        let cfg: EmbeddingProviderConfig = serde_json::from_str(json).unwrap();
        assert!(matches!(cfg, EmbeddingProviderConfig::Ollama(_)));
    }

    #[test]
    fn test_embedding_provider_config_noop_tag() {
        let json = r#"{"type": "noop", "dimension": 4}"#;
        let cfg: EmbeddingProviderConfig = serde_json::from_str(json).unwrap();
        assert!(matches!(cfg, EmbeddingProviderConfig::Noop(_)));
    }

    #[test]
    fn test_embedding_provider_config_http_tag() {
        let json = r#"{"type": "http", "baseUrl": "http://api.example.com", "model": "text-embedding-3-small", "dimension": 1536}"#;
        let cfg: EmbeddingProviderConfig = serde_json::from_str(json).unwrap();
        assert!(matches!(cfg, EmbeddingProviderConfig::Http(_)));
    }

    #[test]
    fn test_vector_config_validate_ok() {
        let json = r#"{
            "provider": {"type": "noop", "dimension": 4},
            "maps": {
                "docs": {"fields": ["title"], "dimension": 4}
            }
        }"#;
        let cfg: VectorConfig = serde_json::from_str(json).unwrap();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_vector_config_validate_mismatch() {
        let json = r#"{
            "provider": {"type": "noop", "dimension": 4},
            "maps": {
                "docs": {"fields": ["title"], "dimension": 8}
            }
        }"#;
        let cfg: VectorConfig = serde_json::from_str(json).unwrap();
        let result = cfg.validate();
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("docs"));
        assert!(msg.contains("8"));
        assert!(msg.contains("4"));
    }

    #[test]
    fn test_http_provider_config_optional_api_key() {
        let json = r#"{"baseUrl": "http://api.example.com", "model": "m", "dimension": 512}"#;
        let cfg: HttpProviderConfig = serde_json::from_str(json).unwrap();
        assert!(cfg.api_key.is_none());
    }

    #[test]
    fn test_map_vector_config_optional_index_name() {
        let json = r#"{"fields": ["body"], "dimension": 4}"#;
        let cfg: MapVectorConfig = serde_json::from_str(json).unwrap();
        assert!(cfg.index_name.is_none());
    }
}
