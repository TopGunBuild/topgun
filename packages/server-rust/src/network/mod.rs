//! Networking types, configuration, connection management, and shutdown control.

pub mod config;
pub mod connection;
pub mod handlers;
pub mod middleware;
pub mod module;
pub mod openapi;
pub mod rate_limit;
pub mod reaper;
pub mod shutdown;

pub use config::*;
pub use connection::*;
pub use handlers::AppState;
pub use module::NetworkModule;
pub use rate_limit::TokenBucket;
pub use reaper::{spawn_connection_reaper, ReaperConfig};
pub use shutdown::*;
