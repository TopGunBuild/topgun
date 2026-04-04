//! External authentication provider trait and types for token exchange.
//!
//! Defines the `AuthProvider` trait and its implementations (`JwksProvider`,
//! `OidcProvider`, `HmacProvider`) for verifying external tokens and extracting
//! claims that can be mapped to `TopGun`'s internal subject + roles format.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use jsonwebtoken::jwk::{AlgorithmParameters, JwkSet};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::RwLock;
use tokio::time::Instant;
use tracing::warn;

/// How long JWKS key sets are cached before a refresh is attempted.
const JWKS_CACHE_TTL: Duration = Duration::from_secs(3600);

/// How long OIDC discovery documents are cached.
const OIDC_DISCOVERY_CACHE_TTL: Duration = Duration::from_secs(86400);

// ── Claim mapping ─────────────────────────────────────────────────────────────

/// Which external claims map to `TopGun`'s `sub` and `roles`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimMapping {
    /// JSON key to read the subject claim from (e.g., "sub", "email", "`user_id`").
    /// Falls back to "sub" when empty.
    #[serde(default)]
    pub sub_claim: String,
    /// JSON key to read the roles claim from.
    /// If absent, the issued `TopGun` JWT has empty roles.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub roles_claim: Option<String>,
}

impl Default for ClaimMapping {
    fn default() -> Self {
        Self {
            sub_claim: "sub".to_string(),
            roles_claim: None,
        }
    }
}

// ── Provider config enum ──────────────────────────────────────────────────────

/// Configuration for a single external auth provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum AuthProviderConfig {
    /// Provider that publishes a JWKS endpoint (Clerk, Auth0, Cognito, etc.).
    Jwks {
        /// Human-readable name for error messages (e.g., "clerk", "auth0").
        name: String,
        /// URL to the JWKS endpoint.
        jwks_url: String,
        /// Expected issuer (`iss` claim). Tokens with a different issuer are rejected.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        issuer: Option<String>,
        /// Expected audience (`aud` claim). Tokens without this audience are rejected.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        audience: Option<String>,
        /// Claim mapping rules.
        #[serde(default)]
        claims: ClaimMapping,
    },
    /// Generic OIDC provider — discovers JWKS URL from `.well-known/openid-configuration`.
    Oidc {
        /// Human-readable name for error messages.
        name: String,
        /// Base URL of the OIDC provider (e.g., `https://accounts.google.com`).
        /// The server appends `/.well-known/openid-configuration` to this URL.
        issuer_url: String,
        /// Expected audience (`aud` claim). Tokens without this audience are rejected.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        audience: Option<String>,
        /// Claim mapping rules.
        #[serde(default)]
        claims: ClaimMapping,
    },
    /// HMAC-based verification for self-signed JWTs (HS256).
    Hmac {
        /// Human-readable name for error messages.
        name: String,
        /// Shared secret for HS256 verification.
        secret: String,
        /// Expected issuer (`iss` claim). Tokens with a different issuer are rejected.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        issuer: Option<String>,
        /// Claim mapping rules.
        #[serde(default)]
        claims: ClaimMapping,
    },
}

// ── External claims ───────────────────────────────────────────────────────────

/// Claims extracted from an external provider's token after successful verification.
#[derive(Debug)]
pub struct ExternalClaims {
    /// Subject identifier mapped from the configured `sub_claim`.
    pub sub: String,
    /// Roles mapped from the configured `roles_claim`. Empty when not configured.
    pub roles: Vec<String>,
}

// ── AuthProvider trait ────────────────────────────────────────────────────────

/// Verifies an external token and extracts claims for `TopGun` JWT issuance.
#[async_trait]
pub trait AuthProvider: Send + Sync {
    /// Provider name used in error messages and logging.
    fn name(&self) -> &str;

    /// Verify an external token and return extracted claims.
    ///
    /// Returns `Err` with a human-readable reason on failure (invalid signature,
    /// expired token, issuer mismatch, etc.).
    async fn verify(&self, token: &str) -> Result<ExternalClaims, String>;
}

// ── Claim extraction helper ───────────────────────────────────────────────────

/// Extract `sub` and `roles` from a decoded JWT payload according to `mapping`.
///
/// `sub_claim` falls back to "sub" when the configured field is empty, guarding
/// against partial `claims` objects that omit the field entirely.
pub(crate) fn extract_claims(payload: &Value, mapping: &ClaimMapping) -> Result<ExternalClaims, String> {
    let sub_key = if mapping.sub_claim.is_empty() {
        "sub"
    } else {
        mapping.sub_claim.as_str()
    };

    let sub = payload
        .get(sub_key)
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .ok_or_else(|| format!("claim '{sub_key}' not found or not a string"))?;

    let roles = match &mapping.roles_claim {
        None => vec![],
        Some(roles_key) => match payload.get(roles_key.as_str()) {
            Some(Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect(),
            Some(Value::String(s)) => vec![s.clone()],
            _ => vec![],
        },
    };

    Ok(ExternalClaims { sub, roles })
}

// ── HMAC provider ─────────────────────────────────────────────────────────────

/// Verifies HS256-signed JWTs using a shared secret.
pub struct HmacProvider {
    provider_name: String,
    secret: String,
    issuer: Option<String>,
    claims: ClaimMapping,
}

impl HmacProvider {
    /// Construct a new `HmacProvider` from its config.
    #[must_use]
    pub fn new(name: String, secret: String, issuer: Option<String>, claims: ClaimMapping) -> Self {
        Self {
            provider_name: name,
            secret,
            issuer,
            claims,
        }
    }
}

#[async_trait]
impl AuthProvider for HmacProvider {
    fn name(&self) -> &str {
        &self.provider_name
    }

    async fn verify(&self, token: &str) -> Result<ExternalClaims, String> {
        let key = DecodingKey::from_secret(self.secret.as_bytes());
        let mut validation = Validation::new(Algorithm::HS256);

        // Skip issuer validation when no issuer is configured; still verify
        // signature and expiry. `iss` defaults to None in Validation, which
        // means no issuer check unless explicitly set.
        if let Some(iss) = &self.issuer {
            validation.set_issuer(&[iss.as_str()]);
        }

        // Audience is not part of the TopGun HMAC token contract for v1.
        validation.validate_aud = false;

        let data = decode::<Value>(token, &key, &validation)
            .map_err(|e| format!("HMAC verification failed: {e}"))?;

        extract_claims(&data.claims, &self.claims)
    }
}

// ── JWKS cache ────────────────────────────────────────────────────────────────

/// A cached JWKS key set with the time it was fetched.
struct CachedJwks {
    jwk_set: JwkSet,
    fetched_at: Instant,
}

// ── JWKS provider ─────────────────────────────────────────────────────────────

/// Verifies JWTs by fetching and caching public keys from a JWKS endpoint.
///
/// Uses a shared `reqwest::Client` to preserve connection pooling across
/// multiple providers.
pub struct JwksProvider {
    provider_name: String,
    jwks_url: String,
    issuer: Option<String>,
    audience: Option<String>,
    claims: ClaimMapping,
    client: Client,
    cache: Arc<RwLock<Option<CachedJwks>>>,
}

impl JwksProvider {
    /// Construct a new `JwksProvider` with a shared HTTP client.
    #[must_use]
    pub fn new(
        name: String,
        jwks_url: String,
        issuer: Option<String>,
        audience: Option<String>,
        claims: ClaimMapping,
        client: Client,
    ) -> Self {
        Self {
            provider_name: name,
            jwks_url,
            issuer,
            audience,
            claims,
            client,
            cache: Arc::new(RwLock::new(None)),
        }
    }

    /// Fetch the JWKS key set, using cache when valid.
    ///
    /// Falls back to stale cache when an HTTP fetch fails so that a transient
    /// JWKS endpoint outage does not break authentication for existing keys.
    async fn get_jwks(&self) -> Result<JwkSet, String> {
        // Fast path: valid cache hit
        {
            let guard = self.cache.read().await;
            if let Some(cached) = guard.as_ref() {
                if cached.fetched_at.elapsed() < JWKS_CACHE_TTL {
                    return Ok(cached.jwk_set.clone());
                }
            }
        }

        // Slow path: fetch from network
        let fetch_result: Result<reqwest::Response, String> = async {
            let resp = self
                .client
                .get(&self.jwks_url)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if resp.status().is_success() {
                Ok(resp)
            } else {
                Err(format!("JWKS fetch returned status {}", resp.status()))
            }
        }
        .await;

        match fetch_result {
            Ok(resp) => {
                let jwk_set: JwkSet = resp
                    .json()
                    .await
                    .map_err(|e| format!("JWKS parse error: {e}"))?;
                let mut guard = self.cache.write().await;
                *guard = Some(CachedJwks {
                    jwk_set: jwk_set.clone(),
                    fetched_at: Instant::now(),
                });
                Ok(jwk_set)
            }
            Err(e) => {
                // Use stale cache rather than failing hard on a transient network error
                let guard = self.cache.read().await;
                if let Some(cached) = guard.as_ref() {
                    warn!(
                        provider = %self.provider_name,
                        error = %e,
                        "JWKS refresh failed; using stale cache"
                    );
                    Ok(cached.jwk_set.clone())
                } else {
                    Err(format!("JWKS fetch failed and no cache available: {e}"))
                }
            }
        }
    }
}

#[async_trait]
impl AuthProvider for JwksProvider {
    fn name(&self) -> &str {
        &self.provider_name
    }

    async fn verify(&self, token: &str) -> Result<ExternalClaims, String> {
        let header = decode_header(token).map_err(|e| format!("invalid token header: {e}"))?;
        let kid = header.kid.ok_or("token header missing 'kid'")?;

        let jwk_set = self.get_jwks().await?;

        let jwk = jwk_set
            .find(&kid)
            .ok_or_else(|| format!("no JWK found for kid '{kid}'"))?;

        let decoding_key = match &jwk.algorithm {
            AlgorithmParameters::RSA(rsa) => DecodingKey::from_rsa_components(&rsa.n, &rsa.e)
                .map_err(|e| format!("invalid RSA key: {e}"))?,
            AlgorithmParameters::EllipticCurve(ec) => {
                DecodingKey::from_ec_components(&ec.x, &ec.y)
                    .map_err(|e| format!("invalid EC key: {e}"))?
            }
            AlgorithmParameters::OctetKeyPair(_) | AlgorithmParameters::OctetKey(_) => {
                return Err("unsupported JWK algorithm type".to_string());
            }
        };

        let key_alg = jwk
            .common
            .key_algorithm
            .ok_or("JWK missing 'alg' field")?;
        let alg = Algorithm::from_str(&key_alg.to_string())
            .map_err(|e| format!("unsupported JWK algorithm: {e}"))?;

        let mut validation = Validation::new(alg);

        if let Some(iss) = &self.issuer {
            validation.set_issuer(&[iss.as_str()]);
        }
        // No issuer configured means no issuer validation (iss defaults to None).

        if let Some(aud) = &self.audience {
            validation.set_audience(&[aud.as_str()]);
        } else {
            validation.validate_aud = false;
        }

        let data = decode::<Value>(token, &decoding_key, &validation)
            .map_err(|e| format!("JWKS token verification failed: {e}"))?;

        extract_claims(&data.claims, &self.claims)
    }
}

// ── OIDC discovery document ───────────────────────────────────────────────────

#[derive(Deserialize)]
struct OidcDiscovery {
    jwks_uri: String,
}

/// A cached OIDC discovery document with time of fetch.
struct CachedDiscovery {
    jwks_uri: String,
    fetched_at: Instant,
}

// ── OIDC provider ─────────────────────────────────────────────────────────────

/// Verifies JWTs from OIDC providers by discovering JWKS URI via
/// `{issuer_url}/.well-known/openid-configuration`.
///
/// After discovery, delegates to a `JwksProvider` constructed with the
/// discovered `jwks_uri` and the same shared `reqwest::Client`.
pub struct OidcProvider {
    provider_name: String,
    issuer_url: String,
    audience: Option<String>,
    claims: ClaimMapping,
    client: Client,
    discovery_cache: Arc<RwLock<Option<CachedDiscovery>>>,
    // Lazily initialized after discovery; wrapped in Arc so concurrent
    // callers can clone a handle without moving the provider out of the lock.
    jwks_provider: Arc<RwLock<Option<Arc<JwksProvider>>>>,
}

impl OidcProvider {
    /// Construct a new `OidcProvider` with a shared HTTP client.
    #[must_use]
    pub fn new(
        name: String,
        issuer_url: String,
        audience: Option<String>,
        claims: ClaimMapping,
        client: Client,
    ) -> Self {
        Self {
            provider_name: name,
            issuer_url,
            audience,
            claims,
            client,
            discovery_cache: Arc::new(RwLock::new(None)),
            jwks_provider: Arc::new(RwLock::new(None)),
        }
    }

    /// Fetch OIDC discovery document and return the `jwks_uri`.
    ///
    /// Caches for 24 hours since OIDC metadata changes very rarely.
    async fn get_jwks_uri(&self) -> Result<String, String> {
        {
            let guard = self.discovery_cache.read().await;
            if let Some(cached) = guard.as_ref() {
                if cached.fetched_at.elapsed() < OIDC_DISCOVERY_CACHE_TTL {
                    return Ok(cached.jwks_uri.clone());
                }
            }
        }

        let discovery_url = format!(
            "{}/.well-known/openid-configuration",
            self.issuer_url.trim_end_matches('/')
        );

        let discovery: OidcDiscovery = self
            .client
            .get(&discovery_url)
            .send()
            .await
            .map_err(|e| format!("OIDC discovery fetch failed: {e}"))?
            .json()
            .await
            .map_err(|e| format!("OIDC discovery parse error: {e}"))?;

        let jwks_uri = discovery.jwks_uri.clone();

        let mut guard = self.discovery_cache.write().await;
        *guard = Some(CachedDiscovery {
            jwks_uri: jwks_uri.clone(),
            fetched_at: Instant::now(),
        });

        Ok(jwks_uri)
    }
}

#[async_trait]
impl AuthProvider for OidcProvider {
    fn name(&self) -> &str {
        &self.provider_name
    }

    async fn verify(&self, token: &str) -> Result<ExternalClaims, String> {
        let jwks_uri = self.get_jwks_uri().await?;

        // Clone the Arc handle so we can release the lock before calling verify.
        // This allows concurrent callers to proceed without seeing None.
        let provider = {
            let needs_rebuild = {
                let guard = self.jwks_provider.read().await;
                guard.as_ref().is_none_or(|p| p.jwks_url != jwks_uri)
            };

            if needs_rebuild {
                let mut guard = self.jwks_provider.write().await;
                // Double-check after acquiring write lock
                if guard.as_ref().is_none_or(|p| p.jwks_url != jwks_uri) {
                    *guard = Some(Arc::new(JwksProvider::new(
                        self.provider_name.clone(),
                        jwks_uri,
                        Some(self.issuer_url.clone()),
                        self.audience.clone(),
                        self.claims.clone(),
                        self.client.clone(),
                    )));
                }
                guard.clone()
            } else {
                self.jwks_provider.read().await.clone()
            }
        };

        match provider {
            Some(p) => p.verify(token).await,
            None => Err("failed to initialize JWKS provider".to_string()),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde_json::json;

    fn make_hmac_token(secret: &str, claims: Value, alg: Algorithm) -> String {
        let header = Header::new(alg);
        encode(&header, &claims, &EncodingKey::from_secret(secret.as_bytes())).unwrap()
    }

    // ── ClaimMapping default ──────────────────────────────────────────────────

    #[test]
    fn claim_mapping_default_sub_is_sub() {
        let m = ClaimMapping::default();
        assert_eq!(m.sub_claim, "sub");
        assert!(m.roles_claim.is_none());
    }

    #[test]
    fn claim_mapping_empty_sub_falls_back() {
        let m = ClaimMapping {
            sub_claim: String::new(),
            roles_claim: None,
        };
        let payload = json!({ "sub": "user-1" });
        let claims = extract_claims(&payload, &m).unwrap();
        assert_eq!(claims.sub, "user-1");
    }

    // ── extract_claims ────────────────────────────────────────────────────────

    #[test]
    fn extract_claims_custom_sub_key() {
        let m = ClaimMapping {
            sub_claim: "email".to_string(),
            roles_claim: None,
        };
        let payload = json!({ "email": "alice@example.com" });
        let claims = extract_claims(&payload, &m).unwrap();
        assert_eq!(claims.sub, "alice@example.com");
        assert!(claims.roles.is_empty());
    }

    #[test]
    fn extract_claims_roles_array() {
        let m = ClaimMapping {
            sub_claim: "sub".to_string(),
            roles_claim: Some("roles".to_string()),
        };
        let payload = json!({ "sub": "u1", "roles": ["admin", "editor"] });
        let claims = extract_claims(&payload, &m).unwrap();
        assert_eq!(claims.roles, vec!["admin", "editor"]);
    }

    #[test]
    fn extract_claims_roles_single_string() {
        let m = ClaimMapping {
            sub_claim: "sub".to_string(),
            roles_claim: Some("role".to_string()),
        };
        let payload = json!({ "sub": "u1", "role": "admin" });
        let claims = extract_claims(&payload, &m).unwrap();
        assert_eq!(claims.roles, vec!["admin"]);
    }

    #[test]
    fn extract_claims_missing_roles_returns_empty() {
        let m = ClaimMapping {
            sub_claim: "sub".to_string(),
            roles_claim: Some("roles".to_string()),
        };
        let payload = json!({ "sub": "u1" });
        let claims = extract_claims(&payload, &m).unwrap();
        assert!(claims.roles.is_empty());
    }

    #[test]
    fn extract_claims_no_roles_claim_configured() {
        let m = ClaimMapping::default();
        let payload = json!({ "sub": "u1", "roles": ["admin"] });
        let claims = extract_claims(&payload, &m).unwrap();
        // roles_claim not configured — should return empty even if field exists
        assert!(claims.roles.is_empty());
    }

    // ── HmacProvider ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn hmac_provider_valid_token() {
        let secret = "test-secret";
        let exp = jsonwebtoken::get_current_timestamp() + 3600;
        let token = make_hmac_token(secret, json!({ "sub": "user-1", "exp": exp }), Algorithm::HS256);

        let provider = HmacProvider::new(
            "test".to_string(),
            secret.to_string(),
            None,
            ClaimMapping::default(),
        );
        let claims = provider.verify(&token).await.unwrap();
        assert_eq!(claims.sub, "user-1");
    }

    #[tokio::test]
    async fn hmac_provider_wrong_secret_fails() {
        let exp = jsonwebtoken::get_current_timestamp() + 3600;
        let token = make_hmac_token("right-secret", json!({ "sub": "u", "exp": exp }), Algorithm::HS256);

        let provider = HmacProvider::new(
            "test".to_string(),
            "wrong-secret".to_string(),
            None,
            ClaimMapping::default(),
        );
        assert!(provider.verify(&token).await.is_err());
    }

    #[tokio::test]
    async fn hmac_provider_expired_token_fails() {
        // Expire far enough in the past to exceed the default 60-second leeway.
        let exp = jsonwebtoken::get_current_timestamp() - 120;
        let token = make_hmac_token("secret", json!({ "sub": "u", "exp": exp }), Algorithm::HS256);

        let provider = HmacProvider::new(
            "test".to_string(),
            "secret".to_string(),
            None,
            ClaimMapping::default(),
        );
        assert!(provider.verify(&token).await.is_err());
    }

    #[tokio::test]
    async fn hmac_provider_issuer_mismatch_fails() {
        let exp = jsonwebtoken::get_current_timestamp() + 3600;
        let token = make_hmac_token(
            "secret",
            json!({ "sub": "u", "iss": "wrong-issuer", "exp": exp }),
            Algorithm::HS256,
        );

        let provider = HmacProvider::new(
            "test".to_string(),
            "secret".to_string(),
            Some("expected-issuer".to_string()),
            ClaimMapping::default(),
        );
        assert!(provider.verify(&token).await.is_err());
    }

    #[tokio::test]
    async fn hmac_provider_issuer_match_succeeds() {
        let exp = jsonwebtoken::get_current_timestamp() + 3600;
        let token = make_hmac_token(
            "secret",
            json!({ "sub": "u", "iss": "my-issuer", "exp": exp }),
            Algorithm::HS256,
        );

        let provider = HmacProvider::new(
            "test".to_string(),
            "secret".to_string(),
            Some("my-issuer".to_string()),
            ClaimMapping::default(),
        );
        let claims = provider.verify(&token).await.unwrap();
        assert_eq!(claims.sub, "u");
    }

    #[tokio::test]
    async fn hmac_provider_roles_extracted() {
        let exp = jsonwebtoken::get_current_timestamp() + 3600;
        let token = make_hmac_token(
            "secret",
            json!({ "sub": "u", "roles": ["admin", "viewer"], "exp": exp }),
            Algorithm::HS256,
        );

        let provider = HmacProvider::new(
            "test".to_string(),
            "secret".to_string(),
            None,
            ClaimMapping {
                sub_claim: "sub".to_string(),
                roles_claim: Some("roles".to_string()),
            },
        );
        let claims = provider.verify(&token).await.unwrap();
        assert_eq!(claims.roles, vec!["admin", "viewer"]);
    }
}
