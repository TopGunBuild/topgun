//! Attribute extraction from `rmpv::Value` records.
//!
//! Records in `TopGun` are stored as `rmpv::Value::Map`. `AttributeExtractor`
//! pulls a named field out of those maps, supporting dot-notation for nested
//! traversal. Each index implementation calls `extract` before indexing.

/// Extracts a named field from an `rmpv::Value` map.
///
/// Supports dot-notation: `"address.city"` traverses the nested map at
/// key `"address"` and then retrieves `"city"`.
///
/// If any segment of the path is missing or the intermediate value is not a
/// map, `rmpv::Value::Nil` is returned.
///
/// Multi-value fields: if the final value is an `rmpv::Value::Array`, the
/// raw array is returned unchanged. Each index implementation is responsible
/// for iterating and expanding the elements individually.
pub struct AttributeExtractor {
    attribute_name: String,
    /// Pre-split path segments for dot-notation traversal.
    segments: Vec<String>,
}

impl AttributeExtractor {
    /// Creates an extractor for the given attribute name.
    ///
    /// Dot-notation is parsed once at construction time, so repeated calls to
    /// `extract` do not allocate for path splitting.
    pub fn new(attribute_name: impl Into<String>) -> Self {
        let name = attribute_name.into();
        let segments = name.split('.').map(String::from).collect();
        AttributeExtractor {
            attribute_name: name,
            segments,
        }
    }

    /// Returns the attribute name this extractor was created for.
    #[must_use]
    pub fn attribute_name(&self) -> &str {
        &self.attribute_name
    }

    /// Extracts the field value from a record.
    ///
    /// `record` is expected to be an `rmpv::Value::Map`. Non-map records
    /// return `Nil` immediately. Uses owned traversal to avoid lifetime
    /// complications when descending nested maps.
    #[must_use]
    pub fn extract(&self, record: &rmpv::Value) -> rmpv::Value {
        let mut current: rmpv::Value = record.clone();
        for segment in &self.segments {
            match current {
                rmpv::Value::Map(entries) => {
                    let found = entries.into_iter().find_map(|(k, v)| {
                        if k.as_str() == Some(segment.as_str()) {
                            Some(v)
                        } else {
                            None
                        }
                    });
                    match found {
                        Some(v) => current = v,
                        None => return rmpv::Value::Nil,
                    }
                }
                _ => return rmpv::Value::Nil,
            }
        }
        current
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_map(pairs: &[(&str, rmpv::Value)]) -> rmpv::Value {
        rmpv::Value::Map(
            pairs
                .iter()
                .map(|(k, v)| (rmpv::Value::String(rmpv::Utf8String::from(*k)), v.clone()))
                .collect(),
        )
    }

    #[test]
    fn flat_field_extraction() {
        let record = make_map(&[
            ("name", rmpv::Value::String(rmpv::Utf8String::from("Alice"))),
            ("age", rmpv::Value::Integer(30.into())),
        ]);
        let extractor = AttributeExtractor::new("name");
        let result = extractor.extract(&record);
        assert_eq!(result, rmpv::Value::String(rmpv::Utf8String::from("Alice")));
    }

    #[test]
    fn nested_field_extraction() {
        let inner = make_map(&[("city", rmpv::Value::String(rmpv::Utf8String::from("NYC")))]);
        let record = make_map(&[("address", inner)]);
        let extractor = AttributeExtractor::new("address.city");
        let result = extractor.extract(&record);
        assert_eq!(result, rmpv::Value::String(rmpv::Utf8String::from("NYC")));
    }

    #[test]
    fn missing_field_returns_nil() {
        let record = make_map(&[("name", rmpv::Value::String(rmpv::Utf8String::from("Bob")))]);
        let extractor = AttributeExtractor::new("age");
        let result = extractor.extract(&record);
        assert_eq!(result, rmpv::Value::Nil);
    }

    #[test]
    fn array_field_returns_raw_array() {
        let tags = rmpv::Value::Array(vec![
            rmpv::Value::String(rmpv::Utf8String::from("rust")),
            rmpv::Value::String(rmpv::Utf8String::from("indexing")),
        ]);
        let record = make_map(&[("tags", tags.clone())]);
        let extractor = AttributeExtractor::new("tags");
        let result = extractor.extract(&record);
        assert_eq!(result, tags);
    }

    #[test]
    fn nested_missing_intermediate_returns_nil() {
        let record = make_map(&[("name", rmpv::Value::String(rmpv::Utf8String::from("Carol")))]);
        let extractor = AttributeExtractor::new("address.city");
        let result = extractor.extract(&record);
        assert_eq!(result, rmpv::Value::Nil);
    }
}
