use std::any::{Any, TypeId};
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use parking_lot::RwLock;

use super::config::ServerConfig;

// ---------------------------------------------------------------------------
// ServiceContext
// ---------------------------------------------------------------------------

/// Context provided to services during initialization.
#[derive(Debug, Clone)]
pub struct ServiceContext {
    pub config: Arc<ServerConfig>,
}

// ---------------------------------------------------------------------------
// ManagedService trait
// ---------------------------------------------------------------------------

/// Lifecycle-managed service trait. All domain services implement this.
///
/// Services are registered with a `ServiceRegistry`, initialized in registration
/// order, and shut down in reverse registration order. The `Any` bound enables
/// type-based lookup via `ServiceRegistry::get::<T>()`.
#[async_trait]
pub trait ManagedService: Send + Sync + Any {
    /// Returns the unique name of this service (e.g., `"crdt"`, `"sync"`).
    fn name(&self) -> &'static str;

    /// Initialize the service with the given context.
    async fn init(&self, ctx: &ServiceContext) -> anyhow::Result<()>;

    /// Reset the service to its initial state (e.g., after partition migration).
    async fn reset(&self) -> anyhow::Result<()>;

    /// Shut down the service. If `terminate` is true, skip graceful cleanup.
    async fn shutdown(&self, terminate: bool) -> anyhow::Result<()>;
}

// ---------------------------------------------------------------------------
// ServiceRegistry
// ---------------------------------------------------------------------------

/// Registry for lifecycle-managed services.
///
/// Provides two lookup mechanisms:
/// - **By name** (`get_by_name`): uses the service's `name()` string
/// - **By type** (`get::<T>`): uses `TypeId` for zero-cost compile-time dispatch
///
/// Services are initialized in registration order and shut down in reverse order.
pub struct ServiceRegistry {
    /// Name-based lookup: service name -> Arc<dyn ManagedService>.
    by_name: DashMap<&'static str, Arc<dyn ManagedService>>,
    /// Type-based lookup: `TypeId` -> `Arc<dyn Any + Send + Sync>`.
    by_type: DashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    /// Registration order for deterministic init/shutdown sequencing.
    init_order: RwLock<Vec<&'static str>>,
}

impl ServiceRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            by_name: DashMap::new(),
            by_type: DashMap::new(),
            init_order: RwLock::new(Vec::new()),
        }
    }

    /// Register a service. The service becomes accessible via both `get::<T>()`
    /// and `get_by_name()`. Registration order determines init/shutdown sequencing.
    pub fn register<T: ManagedService>(&self, service: T) {
        let name = service.name();
        let arc = Arc::new(service);
        self.by_name.insert(name, arc.clone());
        self.by_type.insert(TypeId::of::<T>(), arc);
        self.init_order.write().push(name);
    }

    /// Retrieve a service by its concrete type.
    pub fn get<T: ManagedService>(&self) -> Option<Arc<T>> {
        self.by_type
            .get(&TypeId::of::<T>())
            .and_then(|entry| entry.value().clone().downcast::<T>().ok())
    }

    /// Retrieve a service by its name.
    pub fn get_by_name(&self, name: &str) -> Option<Arc<dyn ManagedService>> {
        self.by_name.get(name).map(|entry| entry.value().clone())
    }

    /// Initialize all registered services in registration order.
    ///
    /// # Errors
    ///
    /// Returns an error if any service's `init()` call fails.
    pub async fn init_all(&self, ctx: &ServiceContext) -> anyhow::Result<()> {
        let order = self.init_order.read().clone();
        for name in &order {
            if let Some(service) = self.get_by_name(name) {
                service.init(ctx).await?;
            }
        }
        Ok(())
    }

    /// Shut down all registered services in reverse registration order.
    ///
    /// # Errors
    ///
    /// Returns an error if any service's `shutdown()` call fails.
    pub async fn shutdown_all(&self, terminate: bool) -> anyhow::Result<()> {
        let order = self.init_order.read().clone();
        for name in order.iter().rev() {
            if let Some(service) = self.get_by_name(name) {
                service.shutdown(terminate).await?;
            }
        }
        Ok(())
    }
}

impl Default for ServiceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    /// Test service that tracks lifecycle calls via atomic counters.
    struct TestService {
        svc_name: &'static str,
        init_counter: AtomicU32,
        shutdown_counter: AtomicU32,
        /// Tracks global init/shutdown ordering across services.
        order_log: Arc<parking_lot::Mutex<Vec<String>>>,
    }

    impl TestService {
        fn new(name: &'static str, order_log: Arc<parking_lot::Mutex<Vec<String>>>) -> Self {
            Self {
                svc_name: name,
                init_counter: AtomicU32::new(0),
                shutdown_counter: AtomicU32::new(0),
                order_log,
            }
        }
    }

    #[async_trait]
    impl ManagedService for TestService {
        fn name(&self) -> &'static str {
            self.svc_name
        }

        async fn init(&self, _ctx: &ServiceContext) -> anyhow::Result<()> {
            self.init_counter.fetch_add(1, Ordering::SeqCst);
            self.order_log
                .lock()
                .push(format!("init:{}", self.svc_name));
            Ok(())
        }

        async fn reset(&self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn shutdown(&self, _terminate: bool) -> anyhow::Result<()> {
            self.shutdown_counter.fetch_add(1, Ordering::SeqCst);
            self.order_log
                .lock()
                .push(format!("shutdown:{}", self.svc_name));
            Ok(())
        }
    }

    /// A distinct service type for type-based lookup testing.
    struct AnotherService;

    #[async_trait]
    impl ManagedService for AnotherService {
        fn name(&self) -> &'static str {
            "another"
        }
        async fn init(&self, _ctx: &ServiceContext) -> anyhow::Result<()> {
            Ok(())
        }
        async fn reset(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn shutdown(&self, _terminate: bool) -> anyhow::Result<()> {
            Ok(())
        }
    }

    fn make_ctx() -> ServiceContext {
        ServiceContext {
            config: Arc::new(ServerConfig::default()),
        }
    }

    #[test]
    fn register_and_get_by_name() {
        let registry = ServiceRegistry::new();
        let log = Arc::new(parking_lot::Mutex::new(Vec::new()));
        registry.register(TestService::new("crdt", log));

        let svc = registry.get_by_name("crdt");
        assert!(svc.is_some());
        assert_eq!(svc.unwrap().name(), "crdt");
    }

    #[test]
    fn get_by_name_unregistered_returns_none() {
        let registry = ServiceRegistry::new();
        assert!(registry.get_by_name("nonexistent").is_none());
    }

    #[test]
    fn register_and_get_by_type() {
        let registry = ServiceRegistry::new();
        registry.register(AnotherService);

        let svc = registry.get::<AnotherService>();
        assert!(svc.is_some());
        assert_eq!(svc.unwrap().name(), "another");
    }

    #[test]
    fn get_by_type_unregistered_returns_none() {
        let registry = ServiceRegistry::new();
        assert!(registry.get::<AnotherService>().is_none());
    }

    #[tokio::test]
    async fn init_all_calls_in_registration_order() {
        let log = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let registry = ServiceRegistry::new();
        registry.register(TestService::new("first", log.clone()));
        registry.register(TestService::new("second", log.clone()));
        registry.register(TestService::new("third", log.clone()));

        let ctx = make_ctx();
        registry.init_all(&ctx).await.unwrap();

        let entries = log.lock().clone();
        assert_eq!(entries, vec!["init:first", "init:second", "init:third"]);
    }

    #[tokio::test]
    async fn shutdown_all_calls_in_reverse_order() {
        let log = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let registry = ServiceRegistry::new();
        registry.register(TestService::new("first", log.clone()));
        registry.register(TestService::new("second", log.clone()));
        registry.register(TestService::new("third", log.clone()));

        registry.shutdown_all(false).await.unwrap();

        let entries = log.lock().clone();
        assert_eq!(
            entries,
            vec!["shutdown:third", "shutdown:second", "shutdown:first"]
        );
    }

    #[test]
    fn register_multiple_services_all_accessible() {
        let log = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let registry = ServiceRegistry::new();
        registry.register(TestService::new("svc-a", log.clone()));
        registry.register(TestService::new("svc-b", log));
        registry.register(AnotherService);

        assert!(registry.get_by_name("svc-a").is_some());
        assert!(registry.get_by_name("svc-b").is_some());
        assert!(registry.get_by_name("another").is_some());
        assert!(registry.get::<AnotherService>().is_some());
    }
}
