//! Operation routing and execution framework.
//!
//! This module implements the service-oriented operation pipeline:
//!
//! 1. **Classification** (`classify`): `Message` -> `Result<Operation, ClassifyError>`
//! 2. **Middleware** (`middleware`): Tower layers (timeout, metrics, load-shedding)
//! 3. **Routing** (`router`): Dispatch to domain services by `service_name`
//! 4. **Domain services** (`domain`): Stub implementations per business domain
//! 5. **Background workers** (`worker`): Periodic tasks (GC, etc.)

pub mod classify;
pub mod config;
pub mod dispatch;
pub mod domain;
pub mod middleware;
pub mod operation;
pub mod policy;
pub mod registry;
pub mod router;
pub mod security;
pub mod worker;

// Re-export key types for convenient access.
pub use classify::OperationService;
pub use config::ServerConfig;
pub use dispatch::PartitionDispatcher;
pub use operation::{
    service_names, CallerOrigin, ClassifyError, Operation, OperationContext, OperationError,
    OperationPipeline, OperationResponse,
};
pub use policy::{
    InMemoryPolicyStore, PermissionAction, PermissionPolicy, PolicyDecision, PolicyEffect,
    PolicyEvaluator, PolicyStore,
};
pub use registry::{ManagedService, ServiceContext, ServiceRegistry};
pub use router::OperationRouter;
pub use security::{SecurityConfig, WriteValidator};
pub use worker::{BackgroundRunnable, BackgroundWorker};
