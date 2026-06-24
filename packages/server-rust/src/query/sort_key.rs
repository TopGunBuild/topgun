//! Comparison key for full-scan pagination with deterministic tie-breaking.
//!
//! [`SortKey`] holds the user-supplied sort-field values for one record plus the
//! record's own primary key (`_key`) as the FINAL mandatory tie-break dimension.
//! Because record primary keys are unique, any two records that compare equal on
//! every user-supplied sort field will still produce a total order that is stable
//! across multiple runs, eviction cycles, and batch boundaries.
//!
//! The tie-break is embedded as a struct field rather than applied as a runtime
//! patch so the [`Ord`] / [`PartialOrd`] implementations always include it — there
//! is no call site that can accidentally forget to apply it.

use topgun_core::messages::base::SortDirection;

// ---------------------------------------------------------------------------
// SortFieldValue
// ---------------------------------------------------------------------------

/// The runtime-typed value of one sort field, extracted from an `rmpv::Value` map.
///
/// Only the primitive types that are meaningfully sortable across page boundaries are
/// represented. Any value that cannot be decoded to one of these variants is treated as
/// `Missing`, which sorts last (after all concrete values) in ascending order and first
/// in descending order — consistent with SQL `NULL LAST` / `NULL FIRST` semantics.
#[derive(Debug, Clone, PartialEq)]
pub enum SortFieldValue {
    /// Numeric value (integer or float); stored as `f64` for uniform comparison.
    Number(f64),
    /// UTF-8 string value.
    Text(String),
    /// Boolean value; `false` < `true`.
    Bool(bool),
    /// Field is absent, `null`, or has a type that cannot participate in ordering.
    Missing,
}

impl SortFieldValue {
    /// Extracts the sort value for `field` from an `rmpv::Value::Map` record.
    ///
    /// Returns [`SortFieldValue::Missing`] when the record is not a map or when
    /// the field is absent.
    #[must_use]
    pub fn extract(field: &str, record: &rmpv::Value) -> Self {
        let rmpv::Value::Map(pairs) = record else {
            return Self::Missing;
        };

        let val = pairs
            .iter()
            .find(|(k, _)| k.as_str() == Some(field))
            .map(|(_, v)| v);

        match val {
            Some(rmpv::Value::Boolean(b)) => Self::Bool(*b),
            Some(rmpv::Value::Integer(i)) => Self::Number(i.as_f64().unwrap_or(f64::NAN)),
            Some(rmpv::Value::F32(f)) => Self::Number(f64::from(*f)),
            Some(rmpv::Value::F64(f)) => Self::Number(*f),
            Some(rmpv::Value::String(s)) => Self::Text(s.as_str().unwrap_or("").to_string()),
            // Absent, null, or complex types (map, array, binary, ext) are not sortable.
            _ => Self::Missing,
        }
    }

    /// Returns a stable integer type-tag for cross-type ordering.
    ///
    /// Missing sorts last; all other types sort before Missing so that absent fields
    /// act like SQL NULLs with NULLS LAST semantics in ascending order.
    fn type_rank(&self) -> u8 {
        match self {
            Self::Bool(_) => 0,
            Self::Number(_) => 1,
            Self::Text(_) => 2,
            Self::Missing => 3,
        }
    }

    /// Three-way comparison between two [`SortFieldValue`]s.
    ///
    /// Returns [`std::cmp::Ordering`] for use in `Ord` implementations.
    #[must_use]
    pub fn cmp_values(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;

        match (self, other) {
            (Self::Number(a), Self::Number(b)) => {
                // NaN is treated as equal to NaN and sorts last among numbers.
                a.partial_cmp(b).unwrap_or(Ordering::Equal)
            }
            (Self::Text(a), Self::Text(b)) => a.cmp(b),
            (Self::Bool(a), Self::Bool(b)) => a.cmp(b),
            // Cross-type or Missing: order by type-rank.
            _ => self.type_rank().cmp(&other.type_rank()),
        }
    }
}

// ---------------------------------------------------------------------------
// SortKey
// ---------------------------------------------------------------------------

/// Comparison key for one record in a sorted full-scan page.
///
/// Holds a parallel list of `(value, direction)` pairs — one per user-supplied sort
/// field — followed by a mandatory `_key` tie-break. The `_key` field is the record's
/// primary key string and is always compared ascending regardless of the query's sort
/// direction. Because primary keys are unique, two `SortKey` values are equal only when
/// they represent the exact same record, which cannot happen in practice.
///
/// ### `BinaryHeap` polarity
///
/// Rust's `BinaryHeap` is a max-heap. Full-scan pagination needs the `limit` BEST rows,
/// defined as those that sort FIRST in the natural page order (ascending by user-supplied
/// sort fields). To achieve this, `Ord` does NOT invert the natural order: the WORST row
/// (the one that would appear last on the page) is "Greater" in `Ord` and therefore sits
/// at the heap root, where it is the first to be evicted when the heap exceeds `limit + 1`.
/// The `_key` tie-break follows the same convention: among equal-valued rows the key with
/// the lexicographically LARGEST value is the "worst" (furthest from the front of the
/// page), so it is Greater and evicted first.
#[derive(Debug, Clone, PartialEq)]
pub struct SortKey {
    /// Sort-field values aligned with the query's `sort` spec.
    ///
    /// Each entry is `(extracted_value, direction)`. The direction is stored here so
    /// `Ord` can apply ASC / DESC semantics without needing the original `Query.sort`
    /// list at comparison time.
    pub fields: Vec<(SortFieldValue, SortDirection)>,

    /// Primary key of the record; mandatory final tie-break dimension.
    ///
    /// Always compared ascending (a record with a lexicographically smaller key sorts
    /// before a record with a larger key when all sort-field values are equal).
    #[allow(clippy::pub_underscore_fields)]
    pub _key: String,
}

impl SortKey {
    /// Builds a [`SortKey`] for one record.
    ///
    /// `sort_spec` must be aligned to `Query.sort`. `record_key` is the record's
    /// primary key.
    #[must_use]
    pub fn new(
        sort_spec: &[(String, SortDirection)],
        record: &rmpv::Value,
        record_key: &str,
    ) -> Self {
        let fields = sort_spec
            .iter()
            .map(|(field, dir)| (SortFieldValue::extract(field, record), dir.clone()))
            .collect();

        Self {
            fields,
            _key: record_key.to_string(),
        }
    }

    /// Compares two sort keys following per-field ASC / DESC semantics, finishing
    /// with the mandatory `_key` tie-break (always ascending).
    ///
    /// Returns the comparison result for the NATURAL order: the record that should
    /// appear FIRST on the result page is `Less`. `Ord` delegates directly to this
    /// function (without inversion) so the `BinaryHeap` root is the WORST row (the
    /// one that sorts LAST on the page) and is the first to be evicted when the heap
    /// exceeds `limit + 1` — see the struct docs on `BinaryHeap` polarity.
    #[must_use]
    #[allow(clippy::used_underscore_binding)]
    pub fn natural_cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;

        for ((a_val, dir), (b_val, _)) in self.fields.iter().zip(other.fields.iter()) {
            let field_cmp = a_val.cmp_values(b_val);
            if field_cmp == Ordering::Equal {
                continue;
            }
            // Invert the comparison result for DESC fields.
            return match dir {
                SortDirection::Asc => field_cmp,
                SortDirection::Desc => field_cmp.reverse(),
            };
        }

        // All sort fields are equal: apply the mandatory _key tie-break (always ASC).
        self._key.cmp(&other._key)
    }
}

/// `Ord` uses the natural comparison so `BinaryHeap<SortKey>` keeps the WORST row
/// at the root (max-heap root = largest = worst row on the page). When the heap
/// overflows `limit + 1`, popping removes the worst row, retaining the best `limit + 1`.
impl Ord for SortKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Natural order: WORSE rows are Greater, so they float to the heap root
        // and are evicted first when the heap exceeds limit + 1.
        self.natural_cmp(other)
    }
}

impl PartialOrd for SortKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for SortKey {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(
    clippy::used_underscore_binding,
    clippy::cast_possible_truncation,
    clippy::doc_markdown
)]
mod tests {
    use std::collections::BinaryHeap;

    use topgun_core::messages::base::SortDirection;

    use super::*;

    fn int_record(field: &str, n: i64) -> rmpv::Value {
        rmpv::Value::Map(vec![(
            rmpv::Value::String(field.into()),
            rmpv::Value::Integer(n.into()),
        )])
    }

    fn str_record(field: &str, s: &str) -> rmpv::Value {
        rmpv::Value::Map(vec![(
            rmpv::Value::String(field.into()),
            rmpv::Value::String(s.into()),
        )])
    }

    fn spec_asc(field: &str) -> Vec<(String, SortDirection)> {
        vec![(field.to_string(), SortDirection::Asc)]
    }

    fn spec_desc(field: &str) -> Vec<(String, SortDirection)> {
        vec![(field.to_string(), SortDirection::Desc)]
    }

    /// `natural_cmp`: smaller integer value is Less in ASC order.
    #[test]
    fn natural_cmp_asc_integer() {
        let a = SortKey::new(&spec_asc("v"), &int_record("v", 10), "ka");
        let b = SortKey::new(&spec_asc("v"), &int_record("v", 20), "kb");
        assert_eq!(a.natural_cmp(&b), std::cmp::Ordering::Less);
        assert_eq!(b.natural_cmp(&a), std::cmp::Ordering::Greater);
    }

    /// `natural_cmp`: LARGER integer value is Less in DESC order (comes first in DESC page).
    #[test]
    fn natural_cmp_desc_integer() {
        let a = SortKey::new(&spec_desc("v"), &int_record("v", 20), "ka");
        let b = SortKey::new(&spec_desc("v"), &int_record("v", 10), "kb");
        assert_eq!(a.natural_cmp(&b), std::cmp::Ordering::Less);
    }

    /// `natural_cmp` tie-break: equal sort values → `_key` ASC decides.
    #[test]
    fn natural_cmp_key_tiebreak_on_equal_sort_value() {
        let a = SortKey::new(&spec_asc("v"), &int_record("v", 5), "key-a");
        let b = SortKey::new(&spec_asc("v"), &int_record("v", 5), "key-b");
        // "key-a" < "key-b" → a is Less (a sorts first on the page).
        assert_eq!(a.natural_cmp(&b), std::cmp::Ordering::Less);
    }

    /// Two `SortKey`s for the same record are Equal.
    #[test]
    fn natural_cmp_same_record_is_equal() {
        let a = SortKey::new(&spec_asc("v"), &int_record("v", 7), "k1");
        let b = SortKey::new(&spec_asc("v"), &int_record("v", 7), "k1");
        assert_eq!(a.natural_cmp(&b), std::cmp::Ordering::Equal);
    }

    /// `BinaryHeap` with inverted `Ord` acts as a min-heap over natural order:
    /// pushing limit+1 entries and popping gives the WORST entry (the one
    /// that sorts LAST on the page).
    #[test]
    fn binary_heap_acts_as_min_heap_for_page_eviction() {
        // ASC sort by "v", limit = 3.
        // Insert values 50, 10, 30, 70, 20 with cap = limit+1 = 4.
        // The heap should retain the 4 best (smallest) values: 10, 20, 30, 50.
        // The root (heap.peek()) is the WORST of the best: 50.
        let spec = spec_asc("v");
        let mut heap: BinaryHeap<SortKey> = BinaryHeap::new();
        let cap = 4; // limit + 1

        for &(v, key) in &[(50, "k5"), (10, "k1"), (30, "k3"), (70, "k7"), (20, "k2")] {
            heap.push(SortKey::new(&spec, &int_record("v", v), key));
            if heap.len() > cap {
                heap.pop(); // evict the worst (largest natural value)
            }
        }

        // Heap has 4 entries: 10, 20, 30, 50. Root is 50 (worst of the best).
        assert_eq!(heap.len(), cap);
        let root = heap.peek().unwrap();
        assert_eq!(root._key, "k5", "root must be the worst (v=50)");

        // Drain into sorted order: pop the worst first, so reverse to get page order.
        let mut page: Vec<i64> = Vec::new();
        while let Some(sk) = heap.pop() {
            if let SortFieldValue::Number(n) = &sk.fields[0].0 {
                page.push(*n as i64);
            }
        }
        page.reverse(); // heap gives worst-first; reverse to get best-first (page order)
        assert_eq!(page, vec![10, 20, 30, 50]);
    }

    /// `_key` tie-break: among entries with the same sort value, keys sort
    /// ascending and the max-heap root is the one with the LARGEST key (worst).
    #[test]
    fn key_tiebreak_in_heap_evicts_largest_key_first() {
        let spec = spec_asc("v");
        let mut heap: BinaryHeap<SortKey> = BinaryHeap::new();
        let cap = 2; // limit + 1

        // All have v=5; keys are "c", "a", "b".
        // Natural order (page order): a < b < c.
        // Heap inverts: heap root = "c" (worst = largest key).
        for key in &["c", "a", "b"] {
            heap.push(SortKey::new(&spec, &int_record("v", 5), key));
            if heap.len() > cap {
                heap.pop();
            }
        }

        // With cap=2, "c" (largest key = worst) is evicted; heap keeps "a" and "b".
        let keys: Vec<_> = heap.iter().map(|sk| sk._key.clone()).collect();
        assert!(keys.contains(&"a".to_string()));
        assert!(keys.contains(&"b".to_string()));
        assert!(!keys.contains(&"c".to_string()), "worst key must be evicted");
    }

    /// `SortFieldValue::extract` handles missing fields gracefully.
    #[test]
    fn extract_missing_field_returns_missing() {
        let rec = int_record("other", 42);
        assert_eq!(SortFieldValue::extract("v", &rec), SortFieldValue::Missing);
    }

    /// `SortFieldValue::extract` handles non-map records gracefully.
    #[test]
    fn extract_from_non_map_record_returns_missing() {
        assert_eq!(
            SortFieldValue::extract("v", &rmpv::Value::Nil),
            SortFieldValue::Missing
        );
    }

    /// String records sort correctly.
    #[test]
    fn natural_cmp_asc_string() {
        let a = SortKey::new(&spec_asc("name"), &str_record("name", "Alice"), "k1");
        let b = SortKey::new(&spec_asc("name"), &str_record("name", "Bob"), "k2");
        assert_eq!(a.natural_cmp(&b), std::cmp::Ordering::Less);
    }
}
