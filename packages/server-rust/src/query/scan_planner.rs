//! Scan plan derivation for full-map query execution.
//!
//! [`ScanPlanner`] translates a [`QueryRequest`] into a [`ScanPlan`] that the
//! full-scan executor consumes: whether a datastore scan is needed, the byte
//! budget for the scan, optional sort and pagination parameters, and whether
//! results should be streamed page-by-page or materialised.
//!
//! This module contains no I/O; the plan is a pure value derived from the
//! query and environment configuration. The executor in the query domain
//! service owns all actual I/O.

use topgun_core::messages::base::Query;

// ---------------------------------------------------------------------------
// Environment-tunable byte budget cap
// ---------------------------------------------------------------------------

/// Default per-scan byte budget when the environment variable is absent.
///
/// Conservative fraction of the default `TOPGUN_MAX_RAM_MB` (1 GiB) ceiling —
/// a single scan page is never allowed to dominate the record cache.
const DEFAULT_SCAN_BUDGET_BYTES: usize = 8 * 1024 * 1024; // 8 MiB

/// Reads the per-scan budget from `TOPGUN_SCAN_BUDGET_BYTES`, falling back to
/// [`DEFAULT_SCAN_BUDGET_BYTES`] on any parse error or absence.
fn env_scan_budget() -> usize {
    std::env::var("TOPGUN_SCAN_BUDGET_BYTES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(DEFAULT_SCAN_BUDGET_BYTES)
}

// ---------------------------------------------------------------------------
// ScanPlan
// ---------------------------------------------------------------------------

/// A fully-resolved plan for one full-map query scan.
///
/// Consumed by the full-scan executor; not stored durably.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanPlan {
    /// Whether a datastore scan is needed to resolve non-resident records.
    ///
    /// `true` when the query may match records that have been evicted from the
    /// in-memory cache; `false` when in-memory residency is sufficient (e.g.
    /// the query has a tight predicate that the index can satisfy entirely from
    /// resident data).
    pub fullscan: bool,

    /// Maximum bytes the scan may hold resident at any point.
    ///
    /// The scan executor splits the datastore enumeration into pages smaller
    /// than this budget so the peak RAM impact stays bounded. Sourced from the
    /// `TOPGUN_SCAN_BUDGET_BYTES` environment variable, defaulting to 8 MiB.
    pub budget_bytes: usize,

    /// Optional sort field for result ordering.
    ///
    /// `None` means the query provided no explicit sort; results will be
    /// returned in arbitrary (implementation-defined) order. See the note
    /// near the LIMIT handling block below for why this matters.
    pub sort_key: Option<String>,

    /// Sort direction: ascending when `true`, descending when `false`.
    /// Ignored when `sort_key` is `None`.
    pub sort_asc: bool,

    /// Optional maximum result count.
    pub limit: Option<usize>,

    /// Optional number of leading results to skip (keyset pagination offset).
    pub offset: Option<usize>,
}

// ---------------------------------------------------------------------------
// ScanPlanner
// ---------------------------------------------------------------------------

/// Derives a [`ScanPlan`] from a [`Query`] and runtime environment.
#[derive(Debug, Default)]
pub struct ScanPlanner;

impl ScanPlanner {
    /// Creates a new planner. Stateless; cheaply cloneable.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Derives a [`ScanPlan`] for the supplied query.
    #[must_use]
    pub fn plan(&self, query: &Query) -> ScanPlan {
        let sort_key = query
            .sort
            .as_ref()
            .and_then(|s| s.first())
            .map(|sf| sf.field.clone());

        let sort_asc = query
            .sort
            .as_ref()
            .and_then(|s| s.first())
            .is_none_or(|sf| {
                matches!(
                    sf.direction,
                    topgun_core::messages::base::SortDirection::Asc
                )
            });

        // DEVELOPER NOTE — LIMIT without an explicit sort key:
        //
        // When a query specifies LIMIT but provides no sort key, the scan
        // returns the first `limit` records in whatever order the datastore
        // enumerates them (typically insertion order or an arbitrary B-tree
        // traversal). This ordering is NOT guaranteed to be stable across
        // restarts, eviction cycles, or redb table compaction. Callers that
        // need stable pagination (i.e. page 2 picks up exactly where page 1
        // left off, with no duplicates or skips) MUST supply an explicit sort
        // key so the keyset cursor can produce a deterministic resumption
        // point. Without a sort key the cursor encodes only the last-key
        // tie-break, which degenerates to an arbitrary order.
        let limit = query.limit.map(|l| l as usize);

        let offset = None; // offset-based pagination is not yet wired; keyset cursors are preferred

        ScanPlan {
            fullscan: true,
            budget_bytes: env_scan_budget(),
            sort_key,
            sort_asc,
            limit,
            offset,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use topgun_core::messages::base::{Query, SortDirection, SortField};

    use super::*;

    fn query_with_sort_and_limit(field: &str, dir: SortDirection, limit: u32) -> Query {
        Query {
            sort: Some(vec![SortField {
                field: field.to_string(),
                direction: dir,
            }]),
            limit: Some(limit),
            ..Default::default()
        }
    }

    fn query_limit_only(limit: u32) -> Query {
        Query {
            limit: Some(limit),
            ..Default::default()
        }
    }

    #[test]
    fn plan_with_sort_and_limit() {
        let planner = ScanPlanner::new();
        let q = query_with_sort_and_limit("score", SortDirection::Desc, 10);
        let plan = planner.plan(&q);

        assert!(plan.fullscan);
        assert_eq!(plan.sort_key, Some("score".to_string()));
        assert!(!plan.sort_asc, "Desc direction should set sort_asc=false");
        assert_eq!(plan.limit, Some(10));
    }

    #[test]
    fn plan_without_sort_key() {
        let planner = ScanPlanner::new();
        let q = query_limit_only(5);
        let plan = planner.plan(&q);

        assert!(plan.fullscan);
        assert_eq!(plan.sort_key, None, "no sort in query → sort_key is None");
        assert_eq!(plan.limit, Some(5));
    }

    #[test]
    fn plan_no_limit_no_sort() {
        let planner = ScanPlanner::new();
        let q = Query::default();
        let plan = planner.plan(&q);

        assert!(plan.fullscan);
        assert_eq!(plan.sort_key, None);
        assert_eq!(plan.limit, None);
    }

    #[test]
    fn plan_budget_is_positive() {
        let plan = ScanPlanner::new().plan(&Query::default());
        assert!(
            plan.budget_bytes > 0,
            "scan budget must always be a positive number of bytes"
        );
    }
}
