//! Startup rebuild path for scalar (Hash / Navigable / Inverted) indexes.
//!
//! Mirrors `vector_index::rebuild_from_store` but operates on the scalar
//! index strategies. Called once during start-up BEFORE `set_ready()` so that
//! queries arriving immediately after readiness see fully populated indexes
//! instead of empty-graph false negatives. The rebuild and the `create_index`
//! backfill read the stored records through
//! [`RecordStoreFactory::scan_merged`], not only the resident ones
//! (`TG-IDX-001`), so start-up time grows with the stored rows of indexed maps.
//!
//! A descriptor whose scan fails logs a `tracing::error!`, leaves its progress
//! entry at `done == false` and the loop continues with the next descriptor.
//! A corrupt single descriptor never aborts the boot.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;

use crate::network::handlers::admin_types::{BackfillProgress, IndexTypeParam, RebuildType};
use crate::service::domain::index::{IndexObserverFactory, IndexRegistry};
use crate::storage::factory::RecordStoreFactory;
use crate::storage::record::RecordValue;

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

/// Fills the index `attribute` of `registry` with every stored record of
/// `map_name` (`TG-IDX-001`), for the startup rebuild and the `create_index`
/// backfill alike.
///
/// - Every row is read through [`RecordStoreFactory::scan_merged`], so stored
///   records count whether or not they are resident.
/// - Every `Lww` row is given to the index with `insert_if_absent`: a write the
///   observer applied during the scan has replaced the key's entry, and an
///   older scanned row of that key must not overwrite it.
/// - `progress.total` is the number of rows scanned so far, of every kind;
///   `progress.processed` the number of `Lww` rows given to the index (whether
///   `insert_if_absent` changed it is not observable). Non-`Lww` rows are not
///   indexed and not counted in `processed`.
/// - `progress.done` becomes `true` only when the scan completes.
///
/// An index removed while the scan runs is skipped row by row; the scan still
/// runs to its end.
///
/// # Errors
///
/// The data-store error of a failed scan. `done` then stays `false`, `total`
/// holds the rows scanned before the error, and the index holds only the rows
/// given to it before the error.
pub(crate) async fn backfill_scalar_index(
    store_factory: &RecordStoreFactory,
    map_name: &str,
    attribute: &str,
    registry: &IndexRegistry,
    progress: &BackfillProgress,
) -> anyhow::Result<u64> {
    store_factory
        .scan_merged(map_name, &mut |key, value| {
            progress.total.fetch_add(1, Ordering::Relaxed);
            if let RecordValue::Lww { value, .. } = value {
                if let Some(index) = registry.get_index(attribute) {
                    index.insert_if_absent(
                        key,
                        &crate::service::domain::predicate::value_to_rmpv(value),
                    );
                    progress.processed.fetch_add(1, Ordering::Relaxed);
                }
            }
        })
        .await?;
    progress.done.store(true, Ordering::Relaxed);
    Ok(progress.total.load(Ordering::Relaxed))
}

/// Re-registers each scalar index named by `specs` and backfills it from the
/// stored records with [`backfill_scalar_index`]. Writes one
/// `BackfillProgress { rebuild_type: StartupRebuild }` entry per descriptor
/// into `backfill_progress` so the admin status endpoint can confirm rebuild
/// completion after `set_ready()`.
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

        match backfill_scalar_index(
            store_factory,
            &spec.map_name,
            &spec.attribute,
            &registry,
            &progress,
        )
        .await
        {
            Ok(total) => tracing::info!(
                target: "topgun_server::index_persistence",
                map = %spec.map_name,
                attribute = %spec.attribute,
                index_type = ?spec.index_type,
                total = total,
                "scalar index rebuild complete"
            ),
            Err(error) => tracing::error!(
                target: "topgun_server::index_persistence",
                map = %spec.map_name,
                attribute = %spec.attribute,
                error = %error,
                "scalar index rebuild failed: the index holds only the records scanned before the error"
            ),
        }
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
        assert!(
            progress.is_empty(),
            "no progress entry should be inserted for a Vector spec"
        );
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
        assert!(index_factory.register_map("users").has_index("email"));
        assert!(index_factory.register_map("events").has_index("kind"));
        assert!(index_factory.register_map("logs").has_index("level"));
    }

    /// Verifies an unknown map name does not abort the loop. The factory's
    /// `register_map` creates an empty registry; the scan visits no row;
    /// rebuild marks done with `total: 0` and continues to the next
    /// descriptor.
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

    /// AC #13 — descriptor-boundary round-trip test.
    ///
    /// Writes scalar descriptors via `save_scalar_descriptors`, reads them
    /// back via `load_scalar_descriptors`, then drives
    /// `rebuild_scalar_from_store` over the resulting specs and asserts both
    /// progress entries and index registration. Exercises the same code path
    /// a real-process restart would, at the descriptor-file boundary, in a
    /// tempdir-backed sidecar so the production sidecar file is untouched.
    /// End-to-end real-process-restart durability is verified manually via
    /// Validation Checklist #1.
    #[tokio::test]
    async fn descriptor_boundary_roundtrip() {
        use crate::network::handlers::admin::{load_scalar_descriptors, save_scalar_descriptors};
        use crate::network::handlers::admin_types::ScalarIndexDescriptor;

        // Sidecar lives in a tempdir so concurrent test runs do not collide on
        // a shared `./scalar_indexes.json` path.
        let dir = tempfile::tempdir().expect("tempdir");
        let sidecar = dir.path().join("scalar_indexes.json");

        let descriptors = vec![
            ScalarIndexDescriptor {
                map_name: "users".to_string(),
                attribute: "email".to_string(),
                index_type: IndexTypeParam::Hash,
                created_at: "2026-05-06T00:00:00Z".to_string(),
            },
            ScalarIndexDescriptor {
                map_name: "events".to_string(),
                attribute: "kind".to_string(),
                index_type: IndexTypeParam::Inverted,
                created_at: "2026-05-06T00:00:01Z".to_string(),
            },
        ];

        // Write -> read -> assert round-trip at the descriptor shape level.
        save_scalar_descriptors(&sidecar, &descriptors).expect("save");
        let loaded = load_scalar_descriptors(&sidecar);
        assert_eq!(loaded.len(), 2, "round-trip preserves descriptor count");
        assert_eq!(loaded[0].map_name, "users");
        assert_eq!(loaded[0].attribute, "email");
        assert_eq!(loaded[1].map_name, "events");
        assert_eq!(loaded[1].attribute, "kind");

        // Drive the rebuild path with the loaded descriptors, then assert the
        // observable post-restart state: progress entries + index registration.
        let (index_factory, store_factory) = fresh_factories();
        let progress: Arc<DashMap<(String, String), Arc<BackfillProgress>>> =
            Arc::new(DashMap::new());
        let specs: Vec<ScalarRebuildSpec> = loaded
            .into_iter()
            .map(|d| ScalarRebuildSpec {
                map_name: d.map_name,
                attribute: d.attribute,
                index_type: d.index_type,
            })
            .collect();
        rebuild_scalar_from_store(&index_factory, &store_factory, &specs, &progress).await;

        assert_eq!(progress.len(), 2);
        assert!(index_factory.register_map("users").has_index("email"));
        assert!(index_factory.register_map("events").has_index("kind"));
        for entry in progress.iter() {
            assert_eq!(entry.value().rebuild_type, RebuildType::StartupRebuild);
            assert!(entry.value().done.load(Ordering::Relaxed));
        }
    }

    // ── TG-IDX-001: backfill over stored records ─────────────────────────

    mod stored_backfill {
        use std::collections::{BTreeMap, HashSet};
        use std::sync::{Arc, Mutex};

        use topgun_core::hlc::Timestamp;
        use topgun_core::types::Value;
        use tracing_subscriber::layer::SubscriberExt as _;

        use super::super::*;
        use crate::storage::factory::test_store::TestStore;
        use crate::storage::impls::StorageConfig;
        use crate::storage::record::OrMapEntry;

        fn record(fields: &[(&str, &str)]) -> RecordValue {
            RecordValue::Lww {
                value: Value::Map(
                    fields
                        .iter()
                        .map(|(k, v)| ((*k).to_string(), Value::String((*v).to_string())))
                        .collect::<BTreeMap<_, _>>(),
                ),
                timestamp: Timestamp {
                    millis: 1_000,
                    counter: 0,
                    node_id: "n".to_string(),
                },
            }
        }

        fn status(value: &str) -> rmpv::Value {
            rmpv::Value::Map(vec![(
                rmpv::Value::from("status"),
                rmpv::Value::from(value),
            )])
        }

        fn keys(names: &[&str]) -> HashSet<String> {
            names.iter().map(ToString::to_string).collect()
        }

        fn progress() -> BackfillProgress {
            BackfillProgress {
                total: AtomicU64::new(0),
                processed: AtomicU64::new(0),
                done: AtomicBool::new(false),
                rebuild_type: RebuildType::Backfill,
            }
        }

        /// The level and fields of every event logged while this layer is the
        /// default subscriber.
        #[derive(Clone, Default)]
        struct Captured(Arc<Mutex<Vec<(tracing::Level, String)>>>);

        struct Fields(String);
        impl tracing::field::Visit for Fields {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                use std::fmt::Write as _;
                let _ = write!(self.0, "{}={value:?} ", field.name());
            }
        }

        impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Captured {
            fn on_event(
                &self,
                event: &tracing::Event<'_>,
                _ctx: tracing_subscriber::layer::Context<'_, S>,
            ) {
                let mut fields = Fields(String::new());
                event.record(&mut fields);
                self.0
                    .lock()
                    .unwrap()
                    .push((*event.metadata().level(), fields.0));
            }
        }

        /// AC24: a key the index already holds under a newer value (a write the
        /// observer applied during the backfill) keeps that value when the scan
        /// meets an older stored row of the key; a non-`Lww` row is neither
        /// indexed nor counted in `processed`; a row without the attribute is
        /// indexed under `Nil`.
        #[tokio::test]
        async fn backfill_keeps_a_newer_entry_and_skips_non_lww_rows() {
            let store = Arc::new(TestStore::new(100));
            store.put_row("m", "k", record(&[("status", "old")]));
            store.put_row("m", "nil", record(&[("other", "x")]));
            store.put_row(
                "m",
                "or",
                RecordValue::OrMap {
                    records: vec![OrMapEntry {
                        value: Value::String("old".to_string()),
                        tag: "t".to_string(),
                        timestamp: Timestamp {
                            millis: 1_000,
                            counter: 0,
                            node_id: "n".to_string(),
                        },
                    }],
                    tombstones: Vec::new(),
                },
            );
            let store_factory =
                RecordStoreFactory::new(StorageConfig::default(), store, Vec::new());
            let index_factory = IndexObserverFactory::new();
            let registry = index_factory.register_map("m");
            registry.add_hash_index("status");
            let index = registry.get_index("status").expect("index registered");
            index.insert("k", &status("new"));
            let progress = progress();

            let scanned =
                backfill_scalar_index(&store_factory, "m", "status", &registry, &progress)
                    .await
                    .expect("the scan succeeds");

            assert_eq!(index.lookup_eq(&rmpv::Value::from("new")), keys(&["k"]));
            assert!(
                index.lookup_eq(&rmpv::Value::from("old")).is_empty(),
                "an older scanned row must not replace the newer entry"
            );
            assert_eq!(index.lookup_eq(&rmpv::Value::Nil), keys(&["nil"]));
            assert_eq!(
                index.entry_count(),
                2,
                "one entry per Lww key, none for the OR row"
            );
            assert_eq!(scanned, 3);
            assert_eq!(
                progress.total.load(Ordering::Relaxed),
                3,
                "total counts every row"
            );
            assert_eq!(
                progress.processed.load(Ordering::Relaxed),
                2,
                "processed counts the Lww rows only"
            );
            assert!(progress.done.load(Ordering::Relaxed));
        }

        /// AC15: a descriptor whose scan fails stays `done == false`, with
        /// `total` at the rows scanned before the error, and logs one ERROR
        /// line; the following descriptor is still processed. The
        /// `create_index` backfill fails the same way.
        #[tokio::test]
        async fn a_failed_scan_leaves_done_false_and_the_next_descriptor_runs() {
            let store = Arc::new(TestStore::new(1));
            store.put_row("bad", "a", record(&[("status", "a")]));
            store.put_row("bad", "b", record(&[("status", "b")]));
            store.fail_resumed_scans("bad");
            store.put_row("good", "a", record(&[("status", "a")]));
            store.put_row("good", "b", record(&[("status", "b")]));
            let store_factory = Arc::new(RecordStoreFactory::new(
                StorageConfig::default(),
                store,
                Vec::new(),
            ));
            let index_factory = Arc::new(IndexObserverFactory::new());
            let backfill: Arc<DashMap<(String, String), Arc<BackfillProgress>>> =
                Arc::new(DashMap::new());
            let specs = ["bad", "good"].map(|map| ScalarRebuildSpec {
                map_name: map.to_string(),
                attribute: "status".to_string(),
                index_type: IndexTypeParam::Hash,
            });

            let captured = Captured::default();
            let guard = tracing::subscriber::set_default(
                tracing_subscriber::registry().with(captured.clone()),
            );
            rebuild_scalar_from_store(&index_factory, &store_factory, &specs, &backfill).await;
            drop(guard);

            let entry = |map: &str| {
                Arc::clone(
                    backfill
                        .get(&(map.to_string(), "status".to_string()))
                        .expect("progress entry")
                        .value(),
                )
            };
            let bad = entry("bad");
            assert!(
                !bad.done.load(Ordering::Relaxed),
                "a failed scan must not report done"
            );
            assert_eq!(
                bad.total.load(Ordering::Relaxed),
                1,
                "total = rows scanned before the error"
            );
            assert_eq!(bad.processed.load(Ordering::Relaxed), 1);
            let good = entry("good");
            assert!(
                good.done.load(Ordering::Relaxed),
                "the next descriptor must still run"
            );
            assert_eq!(good.total.load(Ordering::Relaxed), 2);
            assert_eq!(
                index_factory
                    .register_map("good")
                    .get_index("status")
                    .expect("index")
                    .entry_count(),
                2
            );
            let errors: Vec<String> = captured
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|(level, _)| *level == tracing::Level::ERROR)
                .map(|(_, fields)| fields.clone())
                .collect();
            assert_eq!(
                errors.len(),
                1,
                "one ERROR line for the failed descriptor: {errors:?}"
            );
            assert!(
                errors[0].contains("map=bad") && errors[0].contains("attribute=status"),
                "the ERROR line names the map and the attribute: {}",
                errors[0]
            );

            let registry = index_factory.register_map("bad");
            let progress = progress();
            let result =
                backfill_scalar_index(&store_factory, "bad", "status", &registry, &progress).await;
            assert!(
                result.is_err(),
                "the create_index backfill reports the failed scan"
            );
            assert!(!progress.done.load(Ordering::Relaxed));
            assert_eq!(progress.total.load(Ordering::Relaxed), 1);
        }
    }
}
