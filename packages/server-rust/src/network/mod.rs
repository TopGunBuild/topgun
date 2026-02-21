//! Networking types, configuration, connection management, and shutdown control.

pub mod config;
pub mod connection;
pub mod handlers;
pub mod middleware;
pub mod module;
pub mod shutdown;

pub use config::*;
pub use connection::*;
pub use handlers::AppState;
pub use module::NetworkModule;
pub use shutdown::*;
