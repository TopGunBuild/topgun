//! Tower middleware layers for the operation pipeline.
//!
//! - [`timeout`]: Per-operation timeout enforcement
//! - [`metrics`]: Operation timing and counting via `tracing` spans
//! - [`load_shed`]: Semaphore-based concurrency limiting
//! - [`pipeline`]: Composes all layers into a single service stack
//! - [`observability`]: Tracing subscriber + Prometheus metrics recorder init
//! - [`authorization`]: RBAC policy enforcement (optional)

pub mod authorization;
pub mod load_shed;
pub mod metrics;
pub mod observability;
pub mod pipeline;
pub mod timeout;

pub use authorization::AuthorizationLayer;
pub use load_shed::LoadShedLayer;
pub use metrics::MetricsLayer;
pub use observability::{init_observability, ObservabilityHandle};
pub use pipeline::build_operation_pipeline;
pub use timeout::TimeoutLayer;
