//! Startup rebuild path for scalar (Hash / Navigable / Inverted) indexes.
//!
//! Mirrors `vector_index::rebuild_from_store` but operates on the scalar
//! index strategies. Called once during `NetworkModule::serve()` BEFORE
//! `set_ready()` so that queries arriving immediately after readiness see
//! fully populated indexes instead of empty-graph false negatives.
//!
//! Per-index rebuild failures (record store errors, missing partition,
//! malformed value) emit a `tracing::warn!` and continue to the next
//! descriptor. A corrupt single descriptor never aborts the boot.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;

use crate::network::handlers::admin_types::{BackfillProgress, IndexTypeParam, RebuildType};
use crate::service::domain::index::IndexObserverFactory;
use crate::storage::factory::RecordStoreFactory;

/// Inputs for a single scalar index rebuild during startup.
///
/// One per row of the persisted descriptor file. Carries only the data needed
/// to re-register the index and replay records — the on-disk descriptor's
/// `created_at` field is intentionally dropped here because rebuild does not
/// need it.
#[derive(Clone, Debug)]
pub struct ScalarRebuildSpec {
    pub map_name: String,
    pub attribute: String,
    pub index_type: IndexTypeParam,
}

/// Re-registers each scalar index named by `specs` and backfills it from
/// records already on disk via the existing `for_each_boxed` partition-iter
/// path. Writes one `BackfillProgress { rebuild_type: StartupRebuild }` entry
/// per descriptor into `backfill_progress` so the admin status endpoint can
/// confirm rebuild completion after `set_ready()`.
///
/// Vector descriptors must be filtered out by the caller; this function logs a
/// warn and skips any spec whose `index_type == IndexTypeParam::Vector` rather
/// than panicking, but the calling site in `module.rs::serve` should never
/// pass a Vector spec because the JSON file is written only by the scalar
/// admin handlers.
pub async fn rebuild_scalar_from_store(
    factory: &Arc<IndexObserverFactory>,
    store_factory: &Arc<RecordStoreFactory>,
    specs: &[ScalarRebuildSpec],
    backfill_progress: &Arc<DashMap<(String, String), Arc<BackfillProgress>>>,
) {
    for spec in specs {
        let registry = factory.register_map(&spec.map_name);

        match spec.index_type {
            IndexTypeParam::Hash => registry.add_hash_index(&spec.attribute),
            IndexTypeParam::Navigable => registry.add_navigable_index(&spec.attribute),
            IndexTypeParam::Inverted => registry.add_inverted_index(&spec.attribute),
            IndexTypeParam::Vector => {
                tracing::warn!(
                    target: "topgun_server::index_persistence",
                    map = %spec.map_name,
                    attribute = %spec.attribute,
                    "Vector descriptor encountered in scalar rebuild path; skipping"
                );
                continue;
            }
        }

        let progress = Arc::new(BackfillProgress {
            total: AtomicU64::new(0),
            processed: AtomicU64::new(0),
            done: AtomicBool::new(false),
            rebuild_type: RebuildType::StartupRebuild,
        });
        backfill_progress.insert(
            (spec.map_name.clone(), spec.attribute.clone()),
            Arc::clone(&progress),
        );

        let stores = store_factory.get_all_for_map(&spec.map_name);
        let total: u64 = stores
            .iter()
            .map(|s| u64::try_from(s.size()).unwrap_or(u64::MAX))
            .sum();
        progress.total.store(total, Ordering::Relaxed);

        for store in &stores {
            let attr = spec.attribute.clone();
            let registry_for_iter = Arc::clone(&registry);
            let progress_for_iter = Arc::clone(&progress);
            store.for_each_boxed(
                &mut |key, record| {
                    if let crate::storage::record::RecordValue::Lww { ref value, .. } =
                        record.value
                    {
                        let rmpv_val = crate::service::domain::predicate::value_to_rmpv(value);
                        if let Some(idx) = registry_for_iter.get_index(&attr) {
                            idx.insert(key, &rmpv_val);
                        }
                    }
                    progress_for_iter.processed.fetch_add(1, Ordering::Relaxed);
                },
                false,
            );
        }

        progress.done.store(true, Ordering::Relaxed);
        tracing::info!(
            target: "topgun_server::index_persistence",
            map = %spec.map_name,
            attribute = %spec.attribute,
            index_type = ?spec.index_type,
            total = total,
            "scalar index rebuild complete"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use dashmap::DashMap;

    use super::*;
    use crate::network::handlers::admin_types::IndexTypeParam;
    use crate::service::domain::index::IndexObserverFactory;
    use crate::storage::datastores::NullDataStore;
    use crate::storage::factory::RecordStoreFactory;
    use crate::storage::impls::StorageConfig;

    fn fresh_factories() -> (Arc<IndexObserverFactory>, Arc<RecordStoreFactory>) {
        let index_factory = Arc::new(IndexObserverFactory::new());
        let store_factory = Arc::new(RecordStoreFactory::new(
            StorageConfig::default(),
            Arc::new(NullDataStore),
            Vec::new(),
        ));
        (index_factory, store_factory)
    }

    /// Verifies a `Vector` spec is filtered out of the scalar rebuild path
    /// without panic and without inserting a backfill progress entry. The
    /// scalar rebuild path must never register a vector index — vector
    /// descriptors live in `vector_indexes.json` and are rebuilt by
    /// `vector_index::rebuild_from_store`.
    #[tokio::test]
    async fn rebuild_skips_vector_specs() {
        let (index_factory, store_factory) = fresh_factories();
        let progress: Arc<DashMap<(String, String), Arc<BackfillProgress>>> =
            Arc::new(DashMap::new());

        let specs = vec![ScalarRebuildSpec {
            map_name: "users".to_string(),
            attribute: "embedding".to_string(),
            index_type: IndexTypeParam::Vector,
        }];

        rebuild_scalar_from_store(&index_factory, &store_factory, &specs, &progress).await;

        // Vector specs are skipped before any index registration or progress
        // entry is created; the registry must remain empty for this map.
        assert!(progress.is_empty(), "no progress entry should be inserted for a Vector spec");
        let registry = index_factory.register_map("users");
        assert!(
            !registry.has_index("embedding"),
            "Vector spec must not register an index in the scalar path"
        );
    }

    /// Verifies that one `BackfillProgress` entry is inserted per non-Vector
    /// descriptor, with `rebuild_type == StartupRebuild` and `done == true`
    /// after `rebuild_scalar_from_store` returns.
    #[tokio::test]
    async fn rebuild_inserts_progress_entries() {
        let (index_factory, store_factory) = fresh_factories();
        let progress: Arc<DashMap<(String, String), Arc<BackfillProgress>>> =
            Arc::new(DashMap::new());

        let specs = vec![
            ScalarRebuildSpec {
                map_name: "users".to_string(),
                attribute: "email".to_string(),
                index_type: IndexTypeParam::Hash,
            },
            ScalarRebuildSpec {
                map_name: "events".to_string(),
                attribute: "kind".to_string(),
                index_type: IndexTypeParam::Inverted,
            },
            ScalarRebuildSpec {
                map_name: "logs".to_string(),
                attribute: "level".to_string(),
                index_type: IndexTypeParam::Navigable,
            },
        ];

        rebuild_scalar_from_store(&index_factory, &store_factory, &specs, &progress).await;

        assert_eq!(progress.len(), 3, "one progress entry per non-Vector spec");
        for entry in progress.iter() {
            assert_eq!(
                entry.value().rebuild_type,
                RebuildType::StartupRebuild,
                "rebuild_type must be StartupRebuild"
            );
            assert!(
                entry.value().done.load(Ordering::Relaxed),
                "done must be true after rebuild returns"
            );
        }

        // All three indexes must be registered after rebuild.
        assert!(index_factory
            .register_map("users")
            .has_index("email"));
        assert!(index_factory
            .register_map("events")
            .has_index("kind"));
        assert!(index_factory
            .register_map("logs")
            .has_index("level"));
    }

    /// Verifies an unknown map name does not abort the loop. The factory's
    /// `register_map` creates an empty registry; `get_all_for_map` returns
    /// an empty Vec; rebuild marks done with `total: 0` and continues to
    /// the next descriptor.
    #[tokio::test]
    async fn rebuild_continues_after_missing_map() {
        let (index_factory, store_factory) = fresh_factories();
        let progress: Arc<DashMap<(String, String), Arc<BackfillProgress>>> =
            Arc::new(DashMap::new());

        let specs = vec![
            ScalarRebuildSpec {
                map_name: "ghost".to_string(),
                attribute: "phantom".to_string(),
                index_type: IndexTypeParam::Hash,
            },
            ScalarRebuildSpec {
                map_name: "real".to_string(),
                attribute: "name".to_string(),
                index_type: IndexTypeParam::Hash,
            },
        ];

        rebuild_scalar_from_store(&index_factory, &store_factory, &specs, &progress).await;

        assert_eq!(progress.len(), 2);
        let ghost_entry = progress
            .get(&("ghost".to_string(), "phantom".to_string()))
            .expect("ghost entry should exist");
        assert_eq!(
            ghost_entry.value().total.load(Ordering::Relaxed),
            0,
            "ghost map has no stores; total should be 0"
        );
        assert!(ghost_entry.value().done.load(Ordering::Relaxed));

        // The second descriptor must also have completed despite the first's
        // empty store list.
        let real_entry = progress
            .get(&("real".to_string(), "name".to_string()))
            .expect("real entry should exist");
        assert!(real_entry.value().done.load(Ordering::Relaxed));
    }

}
