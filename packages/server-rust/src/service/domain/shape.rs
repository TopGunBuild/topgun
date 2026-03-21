//! Shape registry for tracking active shapes per connection.
//!
//! `ShapeRegistry` is a `DashMap`-based concurrent data structure that tracks
//! which shapes are active on which connections, following the same pattern
//! as the existing `QueryRegistry`.

use dashmap::DashMap;
use topgun_core::schema::SyncShape;

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// An active shape subscription associated with a connection.
#[derive(Debug, Clone)]
pub struct ActiveShape {
    /// The shape definition (includes map_name, filter, fields, limit).
    pub shape: SyncShape,
    /// The connection that registered this shape.
    pub connection_id: u64,
}

/// Errors that can occur when interacting with the shape registry.
#[derive(Debug, thiserror::Error)]
pub enum ShapeRegistryError {
    /// The shape_id is already registered.
    #[error("Shape ID already registered: {0}")]
    DuplicateShapeId(String),
}

// ---------------------------------------------------------------------------
// ShapeRegistry
// ---------------------------------------------------------------------------

/// Concurrent registry tracking active shapes keyed by shape_id.
///
/// Uses `DashMap` for lock-free concurrent access. Shapes are registered
/// per-connection and can be queried by shape_id, map_name, or connection_id.
pub struct ShapeRegistry {
    shapes: DashMap<String, ActiveShape>,
}

impl ShapeRegistry {
    /// Creates a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            shapes: DashMap::new(),
        }
    }

    /// Registers a shape for a connection.
    ///
    /// Returns an error if a shape with the same `shape_id` is already registered.
    ///
    /// # Errors
    ///
    /// Returns `ShapeRegistryError::DuplicateShapeId` if the shape_id already exists.
    pub fn register(
        &self,
        shape_id: String,
        connection_id: u64,
        shape: SyncShape,
    ) -> Result<(), ShapeRegistryError> {
        use dashmap::mapref::entry::Entry;

        match self.shapes.entry(shape_id.clone()) {
            Entry::Occupied(_) => Err(ShapeRegistryError::DuplicateShapeId(shape_id)),
            Entry::Vacant(entry) => {
                entry.insert(ActiveShape {
                    shape,
                    connection_id,
                });
                Ok(())
            }
        }
    }

    /// Removes and returns a shape by its shape_id.
    pub fn unregister(&self, shape_id: &str) -> Option<ActiveShape> {
        self.shapes.remove(shape_id).map(|(_, v)| v)
    }

    /// Removes all shapes for a given connection, returning the removed shape_ids.
    pub fn unregister_all_for_connection(&self, connection_id: u64) -> Vec<String> {
        let shape_ids: Vec<String> = self
            .shapes
            .iter()
            .filter(|entry| entry.value().connection_id == connection_id)
            .map(|entry| entry.key().clone())
            .collect();

        for id in &shape_ids {
            self.shapes.remove(id);
        }

        shape_ids
    }

    /// Returns all active shapes targeting a specific map.
    ///
    /// Performs a linear scan of all registered shapes. Acceptable for
    /// small shape counts (tens to low hundreds per server).
    #[must_use]
    pub fn shapes_for_map(&self, map_name: &str) -> Vec<(String, ActiveShape)> {
        self.shapes
            .iter()
            .filter(|entry| entry.value().shape.map_name == map_name)
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    /// Returns all shapes for a specific connection.
    #[must_use]
    pub fn shapes_for_connection(&self, connection_id: u64) -> Vec<(String, ActiveShape)> {
        self.shapes
            .iter()
            .filter(|entry| entry.value().connection_id == connection_id)
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    /// Looks up a shape by its shape_id.
    #[must_use]
    pub fn get(&self, shape_id: &str) -> Option<ActiveShape> {
        self.shapes.get(shape_id).map(|entry| entry.value().clone())
    }
}

impl Default for ShapeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a SyncShape with the given map_name.
    fn make_shape(map_name: &str) -> SyncShape {
        SyncShape {
            shape_id: String::new(), // shape_id is tracked by registry key, not struct field
            map_name: map_name.to_string(),
            ..SyncShape::default()
        }
    }

    #[test]
    fn register_and_retrieve() {
        let reg = ShapeRegistry::new();
        let shape = make_shape("users");
        reg.register("s1".into(), 100, shape.clone()).unwrap();

        let active = reg.get("s1").unwrap();
        assert_eq!(active.shape.map_name, "users");
        assert_eq!(active.connection_id, 100);
    }

    #[test]
    fn register_duplicate_returns_error() {
        let reg = ShapeRegistry::new();
        reg.register("s1".into(), 100, make_shape("users")).unwrap();

        let err = reg.register("s1".into(), 200, make_shape("posts")).unwrap_err();
        assert!(
            matches!(err, ShapeRegistryError::DuplicateShapeId(id) if id == "s1")
        );
    }

    #[test]
    fn unregister_returns_removed_shape() {
        let reg = ShapeRegistry::new();
        reg.register("s1".into(), 100, make_shape("users")).unwrap();

        let removed = reg.unregister("s1").unwrap();
        assert_eq!(removed.shape.map_name, "users");
        assert_eq!(removed.connection_id, 100);

        // Should be gone now
        assert!(reg.get("s1").is_none());
    }

    #[test]
    fn unregister_nonexistent_returns_none() {
        let reg = ShapeRegistry::new();
        assert!(reg.unregister("nonexistent").is_none());
    }

    #[test]
    fn unregister_all_for_connection() {
        let reg = ShapeRegistry::new();
        reg.register("s1".into(), 100, make_shape("users")).unwrap();
        reg.register("s2".into(), 100, make_shape("posts")).unwrap();
        reg.register("s3".into(), 200, make_shape("users")).unwrap();

        let mut removed = reg.unregister_all_for_connection(100);
        removed.sort(); // DashMap iteration order is non-deterministic
        assert_eq!(removed, vec!["s1", "s2"]);

        // Connection 100 shapes should be gone
        assert!(reg.get("s1").is_none());
        assert!(reg.get("s2").is_none());

        // Connection 200 shape should remain
        assert!(reg.get("s3").is_some());
    }

    #[test]
    fn shapes_for_map_filters_correctly() {
        let reg = ShapeRegistry::new();
        reg.register("s1".into(), 100, make_shape("users")).unwrap();
        reg.register("s2".into(), 200, make_shape("posts")).unwrap();
        reg.register("s3".into(), 300, make_shape("users")).unwrap();

        let user_shapes = reg.shapes_for_map("users");
        assert_eq!(user_shapes.len(), 2);

        let ids: Vec<&str> = user_shapes.iter().map(|(id, _)| id.as_str()).collect();
        assert!(ids.contains(&"s1"));
        assert!(ids.contains(&"s3"));
    }

    #[test]
    fn shapes_for_connection_filters_correctly() {
        let reg = ShapeRegistry::new();
        reg.register("s1".into(), 100, make_shape("users")).unwrap();
        reg.register("s2".into(), 100, make_shape("posts")).unwrap();
        reg.register("s3".into(), 200, make_shape("users")).unwrap();

        let conn_shapes = reg.shapes_for_connection(100);
        assert_eq!(conn_shapes.len(), 2);

        let ids: Vec<&str> = conn_shapes.iter().map(|(id, _)| id.as_str()).collect();
        assert!(ids.contains(&"s1"));
        assert!(ids.contains(&"s2"));
    }

    #[test]
    fn multiple_shapes_different_connections_same_map() {
        let reg = ShapeRegistry::new();
        reg.register("s1".into(), 100, make_shape("users")).unwrap();
        reg.register("s2".into(), 200, make_shape("users")).unwrap();
        reg.register("s3".into(), 300, make_shape("users")).unwrap();

        let shapes = reg.shapes_for_map("users");
        assert_eq!(shapes.len(), 3);

        // Each should have a different connection_id
        let conn_ids: Vec<u64> = shapes.iter().map(|(_, s)| s.connection_id).collect();
        assert!(conn_ids.contains(&100));
        assert!(conn_ids.contains(&200));
        assert!(conn_ids.contains(&300));
    }
}
