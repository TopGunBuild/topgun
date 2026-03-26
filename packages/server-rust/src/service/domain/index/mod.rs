//! Secondary index subsystem for in-memory predicate acceleration.
//!
//! Provides three index types:
//! - [`HashIndex`]: O(1) equality lookups backed by `DashMap`
//! - [`NavigableIndex`]: O(log N) range queries backed by `BTreeMap`
//! - [`InvertedIndex`]: O(K) token search backed by `DashMap`

pub mod attribute;
pub mod hash_index;
pub mod inverted_index;
pub mod navigable_index;
pub mod registry;

pub use attribute::AttributeExtractor;
pub use hash_index::HashIndex;
pub use inverted_index::InvertedIndex;
pub use navigable_index::NavigableIndex;
pub use registry::{IndexRegistry, IndexStats};

use std::collections::HashSet;

/// Discriminant for the three index strategies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexType {
    Hash,
    Navigable,
    Inverted,
}

/// Core trait implemented by all index types.
///
/// Implementors must be `Send + Sync` to support concurrent access from
/// multiple Tokio tasks without wrapping in an external lock.
pub trait Index: Send + Sync {
    fn index_type(&self) -> IndexType;
    fn attribute_name(&self) -> &str;

    // Mutation hooks -- called by the mutation observer when records change.
    fn insert(&self, key: &str, value: &rmpv::Value);
    fn update(&self, key: &str, old_value: &rmpv::Value, new_value: &rmpv::Value);
    fn remove(&self, key: &str, old_value: &rmpv::Value);
    fn clear(&self);

    // Query methods -- unsupported operations return empty sets rather than
    // panicking, so callers can call any method on any index type safely.
    fn lookup_eq(&self, value: &rmpv::Value) -> HashSet<String>;
    fn lookup_range(
        &self,
        lower: Option<&rmpv::Value>,
        lower_inclusive: bool,
        upper: Option<&rmpv::Value>,
        upper_inclusive: bool,
    ) -> HashSet<String>;
    fn lookup_contains(&self, token: &str) -> HashSet<String>;

    // Stats
    fn entry_count(&self) -> u64;
}

// ---------------------------------------------------------------------------
// IndexableValue
// ---------------------------------------------------------------------------

/// Wraps `rmpv::Value` to provide `Eq` and `Hash` implementations suitable
/// for use as a `HashMap` key.
///
/// `Map` and `Array` variants cannot be hashed deterministically, so they are
/// normalised to `Null` for indexing purposes. Float values are hashed by
/// their bit pattern so that equal-valued floats hash the same; NaN values all
/// hash to the same bucket (all NaN bit patterns are normalised to the
/// canonical quiet-NaN bit pattern).
#[derive(Clone, Debug)]
pub struct IndexableValue(pub rmpv::Value);

impl IndexableValue {
    #[must_use]
    pub fn from_value(v: &rmpv::Value) -> Self {
        match v {
            // Map and Array are not indexable; treat as Null.
            rmpv::Value::Map(_) | rmpv::Value::Array(_) => IndexableValue(rmpv::Value::Nil),
            other => IndexableValue(other.clone()),
        }
    }
}

impl PartialEq for IndexableValue {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (rmpv::Value::Nil, rmpv::Value::Nil) => true,
            (rmpv::Value::Boolean(a), rmpv::Value::Boolean(b)) => a == b,
            (rmpv::Value::Integer(a), rmpv::Value::Integer(b)) => a == b,
            // Floats compared by bit pattern so NaN == NaN for index equality.
            (rmpv::Value::F32(a), rmpv::Value::F32(b)) => a.to_bits() == b.to_bits(),
            (rmpv::Value::F64(a), rmpv::Value::F64(b)) => a.to_bits() == b.to_bits(),
            (rmpv::Value::String(a), rmpv::Value::String(b)) => a == b,
            (rmpv::Value::Binary(a), rmpv::Value::Binary(b)) => a == b,
            // Map/Array were normalised to Nil in the constructor; if somehow
            // present here treat as unequal to everything.
            _ => false,
        }
    }
}

impl Eq for IndexableValue {}

impl std::hash::Hash for IndexableValue {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match &self.0 {
            rmpv::Value::Boolean(b) => {
                1u8.hash(state);
                b.hash(state);
            }
            rmpv::Value::Integer(i) => {
                2u8.hash(state);
                // Hash the canonical i64/u64 representation.
                if let Some(n) = i.as_i64() {
                    n.hash(state);
                } else if let Some(n) = i.as_u64() {
                    n.hash(state);
                }
            }
            rmpv::Value::F32(f) => {
                3u8.hash(state);
                // Normalise NaN to a canonical bit pattern before hashing.
                let bits = if f.is_nan() { f32::NAN.to_bits() } else { f.to_bits() };
                bits.hash(state);
            }
            rmpv::Value::F64(f) => {
                4u8.hash(state);
                let bits = if f.is_nan() { f64::NAN.to_bits() } else { f.to_bits() };
                bits.hash(state);
            }
            rmpv::Value::String(s) => {
                5u8.hash(state);
                s.as_str().hash(state);
            }
            rmpv::Value::Binary(b) => {
                6u8.hash(state);
                b.hash(state);
            }
            // Nil, Map, Array, Ext: all hash to 0 (Map/Array normalised to
            // Nil in the constructor, so they must share its hash bucket).
            rmpv::Value::Nil
            | rmpv::Value::Map(_)
            | rmpv::Value::Array(_)
            | rmpv::Value::Ext(_, _) => 0u8.hash(state),
        }
    }
}

// ---------------------------------------------------------------------------
// ComparableValue
// ---------------------------------------------------------------------------

/// Wraps `rmpv::Value` to provide a total `Ord` implementation suitable for
/// use as a `BTreeMap` key.
///
/// Ordering: Null < Bool < Int < Float < String < Bytes
/// Within each type, values are ordered naturally.
/// Floats use total ordering (`f64::total_cmp`) so NaN sorts deterministically.
#[derive(Clone, Debug)]
pub struct ComparableValue(pub rmpv::Value);

impl PartialEq for ComparableValue {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}

impl Eq for ComparableValue {}

impl ComparableValue {
    #[must_use]
    pub fn from_value(v: &rmpv::Value) -> Self {
        ComparableValue(v.clone())
    }

    fn type_rank(&self) -> u8 {
        match &self.0 {
            rmpv::Value::Nil => 0,
            rmpv::Value::Boolean(_) => 1,
            rmpv::Value::Integer(_) => 2,
            rmpv::Value::F32(_) | rmpv::Value::F64(_) => 3,
            rmpv::Value::String(_) => 4,
            rmpv::Value::Binary(_) => 5,
            // Map/Array/Ext sort after everything else.
            rmpv::Value::Map(_) | rmpv::Value::Array(_) | rmpv::Value::Ext(_, _) => 6,
        }
    }
}

impl PartialOrd for ComparableValue {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ComparableValue {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;

        let rank_cmp = self.type_rank().cmp(&other.type_rank());
        if rank_cmp != Ordering::Equal {
            return rank_cmp;
        }

        match (&self.0, &other.0) {
            (rmpv::Value::Boolean(a), rmpv::Value::Boolean(b)) => a.cmp(b),
            (rmpv::Value::Integer(a), rmpv::Value::Integer(b)) => {
                // Convert to i128 for a unified comparison across signed/unsigned.
                let ai = a.as_i64().map_or_else(|| i128::from(a.as_u64().unwrap_or(0)), i128::from);
                let bi = b.as_i64().map_or_else(|| i128::from(b.as_u64().unwrap_or(0)), i128::from);
                ai.cmp(&bi)
            }
            (rmpv::Value::F32(a), rmpv::Value::F32(b)) => a.total_cmp(b),
            (rmpv::Value::F64(a), rmpv::Value::F64(b)) => a.total_cmp(b),
            // Cross-width float comparison: promote both to f64.
            (rmpv::Value::F32(a), rmpv::Value::F64(b)) => f64::from(*a).total_cmp(b),
            (rmpv::Value::F64(a), rmpv::Value::F32(b)) => a.total_cmp(&f64::from(*b)),
            (rmpv::Value::String(a), rmpv::Value::String(b)) => a.as_str().unwrap_or("").cmp(b.as_str().unwrap_or("")),
            (rmpv::Value::Binary(a), rmpv::Value::Binary(b)) => a.cmp(b),
            _ => Ordering::Equal,
        }
    }
}
