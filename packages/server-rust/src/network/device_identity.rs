//! Server-issued device identity: present-or-mint device credentials.
//!
//! Supplies a server-authenticated, principal-scoped per-replica identity
//! `(principal, deviceId)`. A client presenting a valid credential re-binds its
//! existing `deviceId`; a client with no/invalid/revoked credential is minted a
//! fresh one. Device binding NEVER fails authentication (fail-open-to-new-identity):
//! an attacker can always claim "no token" anyway, so failing an honest user out on
//! cleared site-data / corrupt storage buys no security.
//!
//! At rest only `SHA-256(secret)` (+ issued-at) is stored — never the secret or the
//! opaque token. Verification is a constant-time hash compare. Credentials live under
//! a reserved map namespace kept clear of the user-map namespace by convention.
//!
//! A guest's credential is never stored (`TG-AUTH-002`): it is an HMAC under a key
//! that lives only as long as the process, see [`guest_present_or_mint`].

use std::sync::{Arc, OnceLock};

use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::storage::map_data_store::MapDataStore;
use crate::storage::record::RecordValue;
use topgun_core::hlc::Timestamp;
use topgun_core::types::{Principal, Value};

/// Reserved map namespace for device credentials.
///
/// Kept clear of the user-map namespace by convention (the `_topgun_` prefix).
/// A structural guarantee that no user map can ever claim this name would require
/// reserving the prefix in the map-name validator (`storage/datastores/redb.rs`),
/// which is outside this change's file budget — tracked as a follow-up. The
/// injective `(principal, deviceId)` key below defends intra-namespace collisions.
pub const CREDENTIAL_MAP: &str = "_topgun_device_credentials";

/// Result of a present-or-mint exchange.
#[derive(Clone)]
pub struct DeviceBinding {
    /// The bound device identity (stable across credential rotation).
    pub device_id: String,
    /// A freshly minted/rotated opaque credential — `Some` ONLY when a new
    /// credential was issued (returned to the client in `AUTH_ACK`); `None` on a
    /// plain re-bind of an already-valid presented token.
    pub minted_token: Option<String>,
}

// `Debug` is hand-written (NOT derived) so `minted_token` — which embeds the raw
// 256-bit device secret — can never leak into logs via a stray `{:?}`. Only the
// presence of a credential is shown, never its bytes.
impl std::fmt::Debug for DeviceBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceBinding")
            .field("device_id", &self.device_id)
            .field(
                "minted_token",
                &self.minted_token.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// Credential store over the shared [`MapDataStore`].
///
/// Cheap to construct per handshake (holds one `Arc` clone of the shared store).
pub struct DeviceIdentityStore {
    store: Arc<dyn MapDataStore>,
}

impl DeviceIdentityStore {
    /// Wraps the shared record data store.
    #[must_use]
    pub fn new(store: Arc<dyn MapDataStore>) -> Self {
        Self { store }
    }

    /// Injective, mode-tagged encoding of `(authMode, principal, deviceId)`.
    ///
    /// This is the identity key for BOTH the credential store and (via
    /// [`frontier_client_id`]) the connection-ownership map, so the two agree.
    ///
    /// Two ambiguities must be closed:
    /// 1. A bare `"principal|deviceId"` is NOT injective if a principal may contain
    ///    the delimiter (IdP-issued `sub`s can contain `|`, `:`, etc.). Length-prefixing
    ///    the principal fixes exactly how many following bytes belong to it.
    /// 2. A content-based `NO_AUTH` sentinel is spoofable — a JWT `sub` is an arbitrary
    ///    string and could equal the sentinel text, colliding the two namespaces. So
    ///    the mode is a STRUCTURAL leading tag (`a` = authenticated principal, `n` =
    ///    no-auth) that principal content can never reproduce: a JWT principal always
    ///    encodes through the `a` branch, so no `sub` value can forge an `n` key.
    fn identity_key(principal_id: Option<&str>, device_id: &str) -> String {
        match principal_id {
            Some(p) => format!("a{}:{}|{}", p.len(), p, device_id),
            None => format!("n:{device_id}"),
        }
    }

    /// Split an opaque client token into `(deviceId, secretHex)`.
    ///
    /// Token wire form is `"<deviceId>.<secretHex>"`; the `deviceId` is a UUID
    /// (no `.`) and the secret is hex (no `.`), so a single split is unambiguous.
    fn parse_token(token: &str) -> Option<(&str, &str)> {
        token.split_once('.')
    }

    /// Present-or-mint. NEVER returns an "auth denied" outcome.
    ///
    /// A presented token that parses, targets a UUID `deviceId`, and whose secret
    /// hashes to the stored hash for `(principal, deviceId)` re-binds that identity
    /// with no new credential. Every other case (missing / unparseable / unknown /
    /// revoked / cross-principal / hash-mismatch) mints a fresh identity.
    ///
    /// # Errors
    /// Returns an error only if the underlying store read/write fails; a failed
    /// credential *match* is not an error (it mints fresh — fail-open).
    pub async fn present_or_mint(
        &self,
        principal_id: Option<&str>,
        presented: Option<&str>,
    ) -> anyhow::Result<DeviceBinding> {
        if let Some(token) = presented {
            if let Some((device_id, secret_hex)) = Self::parse_token(token) {
                // A valid UUID deviceId keeps the injective key's second component
                // in a fixed charset and rejects malformed presentations early.
                if Uuid::parse_str(device_id).is_ok() {
                    if let Ok(secret) = hex::decode(secret_hex) {
                        let key = Self::identity_key(principal_id, device_id);
                        if let Some(RecordValue::Lww {
                            value: Value::Bytes(stored_hash),
                            ..
                        }) = self.store.load(CREDENTIAL_MAP, &key).await?
                        {
                            let presented_hash = Sha256::digest(&secret);
                            // Constant-time compare: no early-exit timing signal on
                            // how many leading bytes of the secret hash matched.
                            if bool::from(stored_hash.as_slice().ct_eq(presented_hash.as_slice())) {
                                return Ok(DeviceBinding {
                                    device_id: device_id.to_string(),
                                    minted_token: None,
                                });
                            }
                        }
                    }
                }
            }
        }
        self.mint(principal_id).await
    }

    /// Mint a fresh `(deviceId, secret)`, persist only `SHA-256(secret)`, and return
    /// the opaque token to hand back to the client exactly once.
    async fn mint(&self, principal_id: Option<&str>) -> anyhow::Result<DeviceBinding> {
        let device_id = Uuid::new_v4().to_string();
        let mut secret = [0u8; 32];
        rand::rng().fill_bytes(&mut secret);
        let hash = Sha256::digest(secret);

        let issued_at_ms = now_millis();
        let key = Self::identity_key(principal_id, &device_id);
        // Store the hash as an LWW record; the LWW timestamp millis doubles as the
        // issued-at marker (a credential row is a single-writer server artifact, not
        // a merged CRDT value).
        let record = RecordValue::Lww {
            value: Value::Bytes(hash.to_vec()),
            timestamp: Timestamp {
                millis: issued_at_ms,
                counter: 0,
                node_id: String::new(),
            },
            ttl_ms: None,
        };
        self.store
            .add(
                CREDENTIAL_MAP,
                &key,
                &record,
                i64::try_from(issued_at_ms).unwrap_or(i64::MAX),
            )
            .await?;

        // Opaque to the client: deviceId + hex secret. The secret never touches the
        // store or the logs; only this one return value carries it.
        let token = format!("{}.{}", device_id, hex::encode(secret));
        Ok(DeviceBinding {
            device_id,
            minted_token: Some(token),
        })
    }

    /// Revoke a credential (row delete). The device re-mints on its next connect;
    /// the orphaned `(principal, deviceId)` cursor is reclaimed downstream.
    ///
    /// # Errors
    /// Returns an error if the underlying store delete fails.
    pub async fn revoke(&self, principal_id: Option<&str>, device_id: &str) -> anyhow::Result<()> {
        let key = Self::identity_key(principal_id, device_id);
        self.store.remove(CREDENTIAL_MAP, &key, 0).await
    }
}

/// The key guest device credentials are signed with: 32 random bytes drawn once
/// per process and never persisted or logged, so every guest credential stops
/// verifying when the process ends and the guest is minted a new one
/// (`TG-AUTH-002`).
pub fn guest_credential_key() -> &'static [u8; 32] {
    static KEY: OnceLock<[u8; 32]> = OnceLock::new();
    KEY.get_or_init(|| {
        let mut key = [0u8; 32];
        rand::rng().fill_bytes(&mut key);
        key
    })
}

/// Present-or-mint for a guest, without a store (`TG-AUTH-002`).
///
/// The credential has a user credential's wire shape, `<deviceId>.<64 hex>`, but the
/// hex is `HMAC-SHA-256(key, scope)` over the guest's tenant, kind, id and the
/// `deviceId`, so a presented credential re-binds only under the scope and the key
/// it was minted with; anything else mints fresh, fail-open like
/// [`DeviceIdentityStore::present_or_mint`].
#[must_use]
pub fn guest_present_or_mint(
    key: &[u8; 32],
    principal: &Principal,
    presented: Option<&str>,
) -> DeviceBinding {
    if let Some((device_id, tag_hex)) = presented.and_then(DeviceIdentityStore::parse_token) {
        if Uuid::parse_str(device_id).is_ok() {
            if let Ok(tag) = hex::decode(tag_hex) {
                // `verify_slice` compares in constant time.
                if guest_mac(key, principal, device_id)
                    .verify_slice(&tag)
                    .is_ok()
                {
                    return DeviceBinding {
                        device_id: device_id.to_string(),
                        minted_token: None,
                    };
                }
            }
        }
    }
    let device_id = Uuid::new_v4().to_string();
    let tag = guest_mac(key, principal, &device_id)
        .finalize()
        .into_bytes();
    let token = format!("{device_id}.{}", hex::encode(tag));
    DeviceBinding {
        device_id,
        minted_token: Some(token),
    }
}

/// The MAC of a guest credential. Every field is length-prefixed and the tenant
/// carries a presence tag, so two different scopes never feed the same bytes.
fn guest_mac(key: &[u8; 32], principal: &Principal, device_id: &str) -> Hmac<Sha256> {
    fn field(mac: &mut Hmac<Sha256>, bytes: &[u8]) {
        mac.update(&(bytes.len() as u64).to_be_bytes());
        mac.update(bytes);
    }
    let mut mac =
        <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC accepts a key of any length");
    match &principal.tenant {
        Some(tenant) => {
            mac.update(&[1]);
            field(&mut mac, tenant.as_bytes());
        }
        None => mac.update(&[0]),
    }
    field(&mut mac, principal.kind.as_str().as_bytes());
    field(&mut mac, principal.id.as_bytes());
    field(&mut mac, device_id.as_bytes());
    mac
}

/// Build the server-authenticated frontier identity for a connection.
///
/// Delegates to the same mode-tagged [`DeviceIdentityStore::identity_key`] encoding as
/// the credential store, so the ownership map, the credential keyspace, and (downstream)
/// the tombstone-GC cursor all agree on one key per `(authMode, principal, deviceId)`.
/// The no-auth namespace is a STRUCTURAL tag (`n`), never a content sentinel on
/// `ConnectionMetadata` — `principal` stays `None` there.
#[must_use]
pub fn frontier_client_id(principal_id: Option<&str>, device_id: &str) -> String {
    DeviceIdentityStore::identity_key(principal_id, device_id)
}

/// Wall-clock milliseconds since the Unix epoch.
fn now_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(all(test, feature = "redb"))]
mod tests {
    use super::*;
    use crate::storage::datastores::RedbDataStore;

    async fn mem_store() -> Arc<dyn MapDataStore> {
        // A fresh temp-file redb store per test (unique path avoids cross-test bleed).
        let dir = std::env::temp_dir();
        let mut secret = [0u8; 8];
        rand::rng().fill_bytes(&mut secret);
        let path = dir.join(format!("topgun-devid-{}.redb", hex::encode(secret)));
        let store = RedbDataStore::new(&path).expect("open redb");
        store.initialize().await.expect("init");
        Arc::new(store)
    }

    #[tokio::test]
    async fn mint_on_fresh_then_rebind_on_valid() {
        let store = mem_store().await;
        let s = DeviceIdentityStore::new(store);

        // Fresh: mints a token + deviceId.
        let first = s.present_or_mint(Some("alice"), None).await.unwrap();
        let token = first
            .minted_token
            .clone()
            .expect("fresh mint returns a token");
        assert!(!first.device_id.is_empty());

        // Re-present the exact token: same deviceId, NO new credential.
        let second = s
            .present_or_mint(Some("alice"), Some(&token))
            .await
            .unwrap();
        assert_eq!(
            second.device_id, first.device_id,
            "valid re-present binds same deviceId"
        );
        assert!(
            second.minted_token.is_none(),
            "re-bind issues no new credential"
        );
    }

    #[tokio::test]
    async fn mint_on_garbage_revoked_and_cross_principal_but_never_fails() {
        let store = mem_store().await;
        let s = DeviceIdentityStore::new(store);

        // Garbage token → fresh mint (auth still succeeds).
        let garbage = s
            .present_or_mint(Some("alice"), Some("not-a-token"))
            .await
            .unwrap();
        assert!(garbage.minted_token.is_some());

        // Establish a valid credential for alice.
        let alice = s.present_or_mint(Some("alice"), None).await.unwrap();
        let alice_token = alice.minted_token.unwrap();

        // Same token presented under a DIFFERENT principal → mints fresh (foreign row
        // untouched), never binds cross-account.
        let bob = s
            .present_or_mint(Some("bob"), Some(&alice_token))
            .await
            .unwrap();
        assert!(
            bob.minted_token.is_some(),
            "cross-principal present mints fresh"
        );
        assert_ne!(bob.device_id, alice.device_id);
        // alice's own row still re-binds (was not overwritten by bob's mint).
        let alice_again = s
            .present_or_mint(Some("alice"), Some(&alice_token))
            .await
            .unwrap();
        assert_eq!(alice_again.device_id, alice.device_id);
        assert!(alice_again.minted_token.is_none());

        // Revoked token → mints fresh.
        s.revoke(Some("alice"), &alice.device_id).await.unwrap();
        let after_revoke = s
            .present_or_mint(Some("alice"), Some(&alice_token))
            .await
            .unwrap();
        assert!(
            after_revoke.minted_token.is_some(),
            "revoked present mints fresh"
        );
        assert_ne!(after_revoke.device_id, alice.device_id);
    }

    #[tokio::test]
    async fn hash_at_rest_not_secret() {
        let store = mem_store().await;
        let s = DeviceIdentityStore::new(store.clone());
        let b = s.present_or_mint(Some("alice"), None).await.unwrap();
        let token = b.minted_token.unwrap();
        let (device_id, secret_hex) = token.split_once('.').unwrap();

        // Read the raw stored row: it must be the SHA-256 hash, NOT the secret.
        let key = DeviceIdentityStore::identity_key(Some("alice"), device_id);
        let row = store
            .load(CREDENTIAL_MAP, &key)
            .await
            .unwrap()
            .expect("row exists");
        let RecordValue::Lww {
            value: Value::Bytes(stored),
            ..
        } = row
        else {
            panic!("expected Lww/Bytes credential row");
        };
        let secret = hex::decode(secret_hex).unwrap();
        assert_ne!(stored, secret, "raw secret must never be stored");
        assert_eq!(
            stored,
            Sha256::digest(&secret).to_vec(),
            "stored value is SHA-256(secret)"
        );
        assert_eq!(stored.len(), 32);
    }

    #[test]
    fn identity_key_is_injective_across_delimiter_containing_principals() {
        // ("a|b", "c") vs ("a", "b|c"): a naive "p|d" key collides; the length prefix
        // keeps them distinct.
        let k1 = DeviceIdentityStore::identity_key(Some("a|b"), "c");
        let k2 = DeviceIdentityStore::identity_key(Some("a"), "b|c");
        assert_ne!(k1, k2);
    }

    #[test]
    fn noauth_namespace_cannot_be_forged_by_any_principal_content() {
        // The no-auth namespace is a STRUCTURAL tag (`n`), not a content sentinel: NO
        // principal string — not even one crafted to look like a sentinel — can produce
        // the same key as a genuine no-auth connection, because authenticated principals
        // always encode through the `a` branch.
        let no_auth = frontier_client_id(None, "dev-1");
        for adversarial in ["", "n", "n:dev-1", "\u{0}noauth", "1:x|dev-1"] {
            let with_principal = frontier_client_id(Some(adversarial), "dev-1");
            assert_ne!(
                with_principal, no_auth,
                "principal {adversarial:?} must not collide with the no-auth namespace"
            );
        }
        // Credential-store keying agrees with the frontier/ownership keying.
        assert_eq!(
            no_auth,
            DeviceIdentityStore::identity_key(None, "dev-1"),
            "frontier and credential keys must be the same encoding"
        );
    }

    #[test]
    fn debug_redacts_minted_secret() {
        // A `{:?}` of the binding must NEVER expose the raw token/secret.
        let b = DeviceBinding {
            device_id: "dev-1".to_string(),
            minted_token: Some("dev-1.deadbeefcafe".to_string()),
        };
        let dbg = format!("{b:?}");
        assert!(
            !dbg.contains("deadbeefcafe"),
            "secret leaked via Debug: {dbg}"
        );
        assert!(
            dbg.contains("<redacted>"),
            "Debug must mark the token redacted"
        );
    }

    #[tokio::test]
    async fn secret_and_token_never_logged_on_mint_or_verify() {
        use std::sync::Mutex as StdMutex;
        use tracing_subscriber::layer::SubscriberExt;

        // Capture EVERY field of EVERY event (all levels) emitted while running the
        // mint + verify paths, then assert neither the raw secret hex nor the opaque
        // token ever appears — AC3's "secret/token never appears in logs".
        struct AllFieldsVisitor(String);
        impl tracing::field::Visit for AllFieldsVisitor {
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                use std::fmt::Write as _;
                let _ = write!(self.0, "{}={value} ", field.name());
            }
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                use std::fmt::Write as _;
                let _ = write!(self.0, "{}={value:?} ", field.name());
            }
        }
        #[derive(Clone)]
        struct Capture(Arc<StdMutex<String>>);
        impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Capture {
            fn on_event(
                &self,
                event: &tracing::Event<'_>,
                _ctx: tracing_subscriber::layer::Context<'_, S>,
            ) {
                let mut v = AllFieldsVisitor(String::new());
                event.record(&mut v);
                self.0.lock().unwrap().push_str(&v.0);
            }
        }

        let sink: Arc<StdMutex<String>> = Arc::new(StdMutex::new(String::new()));
        let subscriber = tracing_subscriber::registry().with(Capture(Arc::clone(&sink)));
        let guard = tracing::subscriber::set_default(subscriber);

        let store = mem_store().await;
        let s = DeviceIdentityStore::new(store);
        // Mint path.
        let minted = s.present_or_mint(Some("alice"), None).await.unwrap();
        let token = minted.minted_token.clone().expect("mint returns a token");
        let (_dev, secret_hex) = token.split_once('.').unwrap();
        // Verify/re-bind path.
        let _ = s
            .present_or_mint(Some("alice"), Some(&token))
            .await
            .unwrap();

        drop(guard);
        let logged = sink.lock().unwrap().clone();
        assert!(
            !logged.contains(secret_hex),
            "raw secret hex leaked into logs: {logged}"
        );
        assert!(
            !logged.contains(&token),
            "opaque token leaked into logs: {logged}"
        );
    }
}

// Not behind the `redb` feature like `tests` above: this test needs no store, and
// it must keep compiling in when that feature goes.
#[cfg(test)]
mod guest_credential_tests {
    use super::*;
    use topgun_core::types::PrincipalKind;

    fn principal(tenant: Option<&str>, kind: PrincipalKind, id: &str) -> Principal {
        Principal {
            id: id.to_string(),
            roles: vec!["USER".to_string()],
            tenant: tenant.map(str::to_string),
            kind,
        }
    }

    #[test]
    fn guest_credential_is_stateless_and_rebinds_only_in_its_process() {
        let k1 = [1u8; 32];
        let k2 = [2u8; 32];
        let acme_v1 = principal(Some("acme"), PrincipalKind::Guest, "v1");

        // A first AUTH mints a credential in the user credential's wire shape.
        let first = guest_present_or_mint(&k1, &acme_v1, None);
        let token = first.minted_token.clone().expect("a fresh guest is minted");
        let (device_id, tag) = token.split_once('.').expect("<deviceId>.<hex>");
        assert_eq!(device_id, first.device_id);
        assert!(Uuid::parse_str(device_id).is_ok());
        assert_eq!(tag.len(), 64);
        assert!(tag
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));

        // Re-presented under the same key and scope: the same deviceId, no new token.
        let again = guest_present_or_mint(&k1, &acme_v1, Some(&token));
        assert_eq!(again.device_id, first.device_id);
        assert!(again.minted_token.is_none(), "a re-bind issues no token");

        // Every other presentation mints a new deviceId.
        let remints = |key: &[u8; 32], who: &Principal, presented: &str, why: &str| {
            let b = guest_present_or_mint(key, who, Some(presented));
            assert!(b.minted_token.is_some(), "{why}: must mint");
            assert_ne!(b.device_id, first.device_id, "{why}: must not re-bind");
        };
        remints(
            &k2,
            &acme_v1,
            &token,
            "verified under another key (a restart)",
        );
        remints(
            &k1,
            &principal(Some("globex"), PrincipalKind::Guest, "v1"),
            &token,
            "same sub under another tenant",
        );
        remints(
            &k1,
            &principal(None, PrincipalKind::Guest, "v1"),
            &token,
            "same sub with no tenant",
        );
        remints(
            &k1,
            &principal(Some("acme"), PrincipalKind::Guest, "v2"),
            &token,
            "another guest",
        );
        remints(
            &k1,
            &principal(Some("acme"), PrincipalKind::User, "v1"),
            &token,
            "the same sub as a user",
        );
        // A user credential is `<uuid>.<hex of a random secret>`, which no guest MAC
        // matches.
        let user_credential = format!("{}.{}", Uuid::new_v4(), hex::encode([7u8; 32]));
        remints(&k1, &acme_v1, &user_credential, "a user credential");
        remints(&k1, &acme_v1, "not-a-token", "garbage");
        remints(
            &k1,
            &acme_v1,
            &format!("{}.{tag}", Uuid::new_v4()),
            "the tag moved to another deviceId",
        );
    }
}
