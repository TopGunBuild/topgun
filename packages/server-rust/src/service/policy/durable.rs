//! Durable, write-through [`PolicyStore`] backed by a [`MapDataStore`].
//!
//! Unlike [`InMemoryPolicyStore`](super::InMemoryPolicyStore), whose
//! "configured" marker resets on process restart (the documented fail-open
//! window of the ephemeral backend), this store persists both the policies and
//! the configured-marker through the durable backend. After a restart the
//! gate consults [`is_configured`](PolicyStore::is_configured), which reflects
//! the persisted marker — so a deployment that ever adopted RBAC keeps
//! enforcing default-deny instead of silently reverting to allow-all.
//!
//! # Persistence shape
//!
//! Everything lives under one reserved map (`__topgun_policies`). The backend's
//! [`RecordValue`] is CRDT-shaped and has no opaque-blob variant, so each
//! payload is msgpack-encoded (`rmp_serde::to_vec_named`) and wrapped in a
//! [`RecordValue::Lww`] with a synthetic [`Timestamp`]. The same backend wire
//! format the redb/Postgres stores already use is reused — no new record
//! variant, no second serialization path.
//!
//! Three reserved keys are used:
//! - `<policy id>`  — one record per policy.
//! - `__configured` — the sticky marker. Set on first upsert, NEVER cleared on
//!   delete. Its presence on load sets `is_configured() == true`.
//! - `__index`      — the list of live policy ids, so the loader can enumerate
//!   policies via `load_all` (the backend has no "list all keys" primitive).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use topgun_core::hlc::Timestamp;
use topgun_core::types::Value;

use super::{glob_matches, PermissionPolicy, PolicyError, PolicyStore};
use crate::storage::map_data_store::MapDataStore;
use crate::storage::record::RecordValue;

/// Reserved internal map under which all policy persistence lives.
///
/// Starts with `_`, so it satisfies the backend's `^[a-zA-Z_][a-zA-Z0-9_]*$`
/// map-name validation, and is namespaced to avoid colliding with user maps.
const POLICY_MAP: &str = "__topgun_policies";

/// Reserved key holding the sticky configured-marker.
const CONFIGURED_KEY: &str = "__configured";

/// Reserved key holding the msgpack-encoded `Vec<String>` of live policy ids.
const INDEX_KEY: &str = "__index";

/// Fixed sentinel node id for synthetic timestamps on persisted policy records.
///
/// These records are not CRDT values from any real node; the timestamp exists
/// only to satisfy the `RecordValue::Lww` shape. A fixed sentinel keeps the
/// encoding deterministic and clearly non-clobbering of real node ids.
const SENTINEL_NODE_ID: &str = "__policy_store";

/// Build a synthetic timestamp for wrapping a policy payload.
fn sentinel_timestamp() -> Timestamp {
    Timestamp {
        millis: 0,
        counter: 0,
        node_id: SENTINEL_NODE_ID.to_string(),
    }
}

/// Wrap raw msgpack bytes into the backend's CRDT-shaped record value.
fn wrap_bytes(bytes: Vec<u8>) -> RecordValue {
    RecordValue::Lww {
        value: Value::Bytes(bytes),
        timestamp: sentinel_timestamp(),
    }
}

/// Reverse [`wrap_bytes`]: extract the raw msgpack bytes from a loaded record.
fn unwrap_bytes(value: &RecordValue) -> Result<&[u8], PolicyError> {
    match value {
        RecordValue::Lww {
            value: Value::Bytes(b),
            ..
        } => Ok(b),
        other => Err(PolicyError::Internal(format!(
            "policy record has unexpected shape: {other:?}"
        ))),
    }
}

/// Durable, write-through policy store over an `Arc<dyn MapDataStore>`.
///
/// Reads are served from an in-memory cache populated at startup by
/// [`load_from_backend`](Self::load_from_backend); writes persist through to
/// the backend (and durably set the marker) BEFORE updating the cache and
/// returning `Ok(())`.
pub struct DurablePolicyStore {
    backend: Arc<dyn MapDataStore>,
    /// In-memory read cache, keyed by policy id. Authoritative for reads after
    /// `load_from_backend`; kept in sync on every successful durable write.
    cache: DashMap<String, PermissionPolicy>,
    /// Mirrors the persisted configured-marker. Once true, never reset within
    /// the process. On load it is set from the marker's presence in the backend.
    configured: AtomicBool,
}

impl DurablePolicyStore {
    /// Create a durable store over the given backend.
    ///
    /// The cache starts empty; call [`load_from_backend`](Self::load_from_backend)
    /// once at startup to hydrate it from the durable backend before serving.
    #[must_use]
    pub fn new(backend: Arc<dyn MapDataStore>) -> Self {
        Self {
            backend,
            cache: DashMap::new(),
            configured: AtomicBool::new(false),
        }
    }

    /// Persist the current list of live policy ids so the loader can enumerate
    /// them after a restart (the backend has no "list all keys" primitive).
    async fn persist_index(&self) -> Result<(), PolicyError> {
        let ids: Vec<String> = self.cache.iter().map(|e| e.key().clone()).collect();
        let bytes = rmp_serde::to_vec_named(&ids)
            .map_err(|e| PolicyError::Internal(format!("encode policy index: {e}")))?;
        self.backend
            .add(POLICY_MAP, INDEX_KEY, &wrap_bytes(bytes), 0, 0)
            .await
            .map_err(|e| PolicyError::Internal(format!("persist policy index: {e}")))
    }

    /// Durably set the sticky configured-marker. Idempotent.
    async fn persist_marker(&self) -> Result<(), PolicyError> {
        // The marker payload is irrelevant; only its presence matters. Encode a
        // trivial sentinel so the record is well-formed for the backend.
        let bytes = rmp_serde::to_vec_named(&true)
            .map_err(|e| PolicyError::Internal(format!("encode marker: {e}")))?;
        self.backend
            .add(POLICY_MAP, CONFIGURED_KEY, &wrap_bytes(bytes), 0, 0)
            .await
            .map_err(|e| PolicyError::Internal(format!("persist configured marker: {e}")))
    }

    /// Hydrate the in-memory cache and configured-marker from the backend.
    ///
    /// Called once at startup. Fails closed: if the marker is present (the
    /// deployment HAS configured RBAC) but the policy records cannot be read or
    /// deserialized, this returns `Err` rather than silently leaving an empty
    /// cache — the binary treats that as fatal so a backend fault cannot
    /// downgrade a configured store to allow-all.
    ///
    /// # Errors
    ///
    /// Returns `PolicyError::Internal` if the backend errors while the marker
    /// says configured, or if any indexed policy record fails to load/decode.
    pub async fn load_from_backend(&self) -> Result<(), PolicyError> {
        // 1. Determine configured state from the marker's presence.
        let marker = self
            .backend
            .load(POLICY_MAP, CONFIGURED_KEY)
            .await
            .map_err(|e| PolicyError::Internal(format!("load configured marker: {e}")))?;
        let is_configured = marker.is_some();
        self.configured.store(is_configured, Ordering::SeqCst);

        // A never-configured store has nothing to load; empty cache is correct.
        if !is_configured {
            return Ok(());
        }

        // 2. Read the policy-id index. While configured, an unreadable index is
        //    a fail-closed condition, not "no policies".
        let index_record = self
            .backend
            .load(POLICY_MAP, INDEX_KEY)
            .await
            .map_err(|e| PolicyError::Internal(format!("load policy index: {e}")))?;

        let ids: Vec<String> = match index_record {
            Some(rec) => {
                let bytes = unwrap_bytes(&rec)?;
                rmp_serde::from_slice(bytes)
                    .map_err(|e| PolicyError::Internal(format!("decode policy index: {e}")))?
            }
            // Configured but no index: a configured-then-emptied store. No
            // policies to load, but the marker keeps default-deny in force.
            None => Vec::new(),
        };

        if ids.is_empty() {
            return Ok(());
        }

        // 3. Load every indexed policy. A missing or undecodable record while
        //    configured is fail-closed.
        let loaded = self
            .backend
            .load_all(POLICY_MAP, &ids)
            .await
            .map_err(|e| PolicyError::Internal(format!("load policies: {e}")))?;

        let mut by_key: std::collections::HashMap<String, RecordValue> =
            loaded.into_iter().collect();
        for id in &ids {
            let rec = by_key.remove(id).ok_or_else(|| {
                PolicyError::Internal(format!(
                    "indexed policy '{id}' missing from backend (fail closed)"
                ))
            })?;
            let bytes = unwrap_bytes(&rec)?;
            let policy: PermissionPolicy = rmp_serde::from_slice(bytes)
                .map_err(|e| PolicyError::Internal(format!("decode policy '{id}': {e}")))?;
            self.cache.insert(id.clone(), policy);
        }

        Ok(())
    }
}

#[async_trait]
impl PolicyStore for DurablePolicyStore {
    async fn get_policies(&self, map_name: &str) -> Result<Vec<PermissionPolicy>, PolicyError> {
        Ok(self
            .cache
            .iter()
            .filter(|entry| glob_matches(&entry.map_pattern, map_name))
            .map(|entry| entry.value().clone())
            .collect())
    }

    async fn list_policies(&self) -> Result<Vec<PermissionPolicy>, PolicyError> {
        Ok(self
            .cache
            .iter()
            .map(|entry| entry.value().clone())
            .collect())
    }

    async fn upsert_policy(&self, policy: PermissionPolicy) -> Result<(), PolicyError> {
        // Write-through: persist the policy and durably set the marker BEFORE
        // touching the cache or returning Ok (append-before-ack parity).
        let bytes = rmp_serde::to_vec_named(&policy)
            .map_err(|e| PolicyError::Internal(format!("encode policy: {e}")))?;
        self.backend
            .add(POLICY_MAP, &policy.id, &wrap_bytes(bytes), 0, 0)
            .await
            .map_err(|e| PolicyError::Internal(format!("persist policy: {e}")))?;

        // Set the durable marker first time round; idempotent thereafter.
        self.persist_marker().await?;

        // Update the cache, then refresh the durable index from it.
        self.cache.insert(policy.id.clone(), policy);
        self.persist_index().await?;

        self.configured.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn delete_policy(&self, policy_id: &str) -> Result<(), PolicyError> {
        // Write-through delete. The configured-marker is intentionally NEVER
        // cleared here: a configured-then-emptied store must keep default-deny.
        self.backend
            .remove(POLICY_MAP, policy_id, 0)
            .await
            .map_err(|e| PolicyError::Internal(format!("delete policy: {e}")))?;

        self.cache.remove(policy_id);
        self.persist_index().await?;
        Ok(())
    }

    async fn is_configured(&self) -> Result<bool, PolicyError> {
        Ok(self.configured.load(Ordering::SeqCst))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::policy::{
        GateDecision, InMemoryPolicyStore, PermissionAction, PolicyDecision, PolicyEffect,
        PolicyEvaluator,
    };
    #[cfg(feature = "redb")]
    use crate::storage::datastores::RedbDataStore;
    #[cfg(feature = "redb")]
    use tempfile::TempDir;

    fn deny_read_policy(map_pattern: &str) -> PermissionPolicy {
        PermissionPolicy {
            id: "deny-read".to_string(),
            map_pattern: map_pattern.to_string(),
            action: PermissionAction::Read,
            effect: PolicyEffect::Deny,
            condition: None,
        }
    }

    #[cfg(feature = "redb")]
    fn open_redb(dir: &TempDir, file: &str) -> Arc<RedbDataStore> {
        let path = dir.path().join(file);
        Arc::new(RedbDataStore::new(&path).expect("open redb"))
    }

    // -- AC1: round-trip through a fresh store over the same file --

    #[cfg(feature = "redb")]
    #[tokio::test]
    async fn ac1_upsert_then_reload_lists_policy() {
        let dir = TempDir::new().unwrap();
        {
            let store = DurablePolicyStore::new(open_redb(&dir, "ac1.redb"));
            store
                .upsert_policy(deny_read_policy("users.*"))
                .await
                .unwrap();
        }
        // Fresh store over the SAME file.
        let reopened = DurablePolicyStore::new(open_redb(&dir, "ac1.redb"));
        reopened.load_from_backend().await.unwrap();
        let all = reopened.list_policies().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "deny-read");
    }

    // -- AC2 (behavioral): a Deny survives restart and the evaluator denies --

    #[cfg(feature = "redb")]
    #[tokio::test]
    async fn ac2_deny_policy_survives_restart_and_denies() {
        let dir = TempDir::new().unwrap();
        {
            let store = DurablePolicyStore::new(open_redb(&dir, "ac2.redb"));
            store.upsert_policy(deny_read_policy("docs")).await.unwrap();
            // store dropped here -> simulated restart
        }

        let reopened = Arc::new(DurablePolicyStore::new(open_redb(&dir, "ac2.redb")));
        reopened.load_from_backend().await.unwrap();

        // The policy is present after the restart.
        let present = reopened.get_policies("docs").await.unwrap();
        assert_eq!(present.len(), 1, "deny policy must survive restart");

        // And the evaluator actually denies a READ on the protected map.
        let eval = PolicyEvaluator::new(reopened.clone());
        let user = topgun_core::Principal {
            id: "u1".to_string(),
            roles: vec!["user".to_string()],
        };
        let decision = eval
            .evaluate(
                Some(&user),
                PermissionAction::Read,
                "docs",
                &rmpv::Value::Nil,
            )
            .await;
        assert_eq!(decision, PolicyDecision::Deny);
    }

    // -- AC3 (negative control): the non-durable store loses configuration --

    #[tokio::test]
    async fn ac3_inmemory_store_loses_configuration_across_restart() {
        // "Before restart": configure an in-memory store.
        {
            let store = InMemoryPolicyStore::new();
            store.upsert_policy(deny_read_policy("docs")).await.unwrap();
            assert!(store.is_configured().await.unwrap());
        }
        // "After restart": a brand-new in-memory store has no persisted state.
        let after = Arc::new(InMemoryPolicyStore::new());
        assert!(
            !after.is_configured().await.unwrap(),
            "ephemeral store must lose the configured marker on restart"
        );
        let eval = PolicyEvaluator::new(after);
        assert_eq!(
            eval.should_evaluate().await,
            GateDecision::AllowAll,
            "lost configuration -> gate opens (proves AC2 discriminates durability)"
        );
    }

    // -- AC4 (fail-closed): marker present but load_all errors -> Err --

    /// Fake backend: reports the configured marker and an index referencing one
    /// policy, but errors on `load_all`. Exercises the fail-closed path.
    struct FailingLoadStore;

    #[async_trait]
    impl MapDataStore for FailingLoadStore {
        async fn add(
            &self,
            _m: &str,
            _k: &str,
            _v: &RecordValue,
            _e: i64,
            _n: i64,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn add_backup(
            &self,
            _m: &str,
            _k: &str,
            _v: &RecordValue,
            _e: i64,
            _n: i64,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn remove(&self, _m: &str, _k: &str, _n: i64) -> anyhow::Result<()> {
            Ok(())
        }
        async fn remove_backup(&self, _m: &str, _k: &str, _n: i64) -> anyhow::Result<()> {
            Ok(())
        }
        async fn load(&self, _m: &str, key: &str) -> anyhow::Result<Option<RecordValue>> {
            // Marker present -> store is configured.
            if key == CONFIGURED_KEY {
                let bytes = rmp_serde::to_vec_named(&true)?;
                return Ok(Some(wrap_bytes(bytes)));
            }
            // Index references one policy id so load_all is reached.
            if key == INDEX_KEY {
                let bytes = rmp_serde::to_vec_named(&vec!["deny-read".to_string()])?;
                return Ok(Some(wrap_bytes(bytes)));
            }
            Ok(None)
        }
        async fn load_all(
            &self,
            _m: &str,
            _keys: &[String],
        ) -> anyhow::Result<Vec<(String, RecordValue)>> {
            anyhow::bail!("simulated backend failure")
        }
        async fn remove_all(&self, _m: &str, _keys: &[String]) -> anyhow::Result<()> {
            Ok(())
        }
        fn is_loadable(&self, _key: &str) -> bool {
            true
        }
        fn pending_operation_count(&self) -> u64 {
            0
        }
        async fn soft_flush(&self) -> anyhow::Result<u64> {
            Ok(0)
        }
        async fn hard_flush(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn flush_key(
            &self,
            _m: &str,
            _k: &str,
            _v: &RecordValue,
            _b: bool,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        fn reset(&self) {}
    }

    #[tokio::test]
    async fn ac4_load_error_while_configured_fails_closed() {
        let store = DurablePolicyStore::new(Arc::new(FailingLoadStore));
        let result = store.load_from_backend().await;
        assert!(
            result.is_err(),
            "marker present + load_all error must return Err, not a silent empty cache"
        );
        // Cache must not be silently populated.
        assert!(store.list_policies().await.unwrap().is_empty());
    }

    // -- AC6 (configured-but-empty): delete all -> stays configured -> Evaluate --

    #[cfg(feature = "redb")]
    #[tokio::test]
    async fn ac6_configured_but_empty_still_evaluates() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(DurablePolicyStore::new(open_redb(&dir, "ac6.redb")));
        store.upsert_policy(deny_read_policy("docs")).await.unwrap();
        store.delete_policy("deny-read").await.unwrap();

        assert!(
            store.is_configured().await.unwrap(),
            "delete must NOT clear the configured marker"
        );
        assert!(store.list_policies().await.unwrap().is_empty());

        let eval = PolicyEvaluator::new(store.clone());
        assert_eq!(
            eval.should_evaluate().await,
            GateDecision::Evaluate,
            "configured-but-empty must Evaluate (default-deny), not AllowAll"
        );

        // Release the redb file lock before reopening (single-writer backend).
        drop(eval);
        drop(store);

        // And the durable marker survives a restart while policies are empty.
        let reopened = Arc::new(DurablePolicyStore::new(open_redb(&dir, "ac6.redb")));
        reopened.load_from_backend().await.unwrap();
        assert!(reopened.is_configured().await.unwrap());
        assert!(reopened.list_policies().await.unwrap().is_empty());
        let eval2 = PolicyEvaluator::new(reopened);
        assert_eq!(eval2.should_evaluate().await, GateDecision::Evaluate);
    }
}
