// Stub file -- will be fully implemented in G1-S2 (Operation types segment).

/// Origin of the operation caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerOrigin {
    Client,
    Forwarded,
    Backup,
    Wan,
    System,
}

/// Context carried with every operation through the pipeline.
#[derive(Debug, Clone)]
pub struct OperationContext {
    pub call_id: u64,
    pub partition_id: Option<u32>,
    pub service_name: &'static str,
    pub caller_origin: CallerOrigin,
    pub client_id: Option<String>,
    pub caller_node_id: Option<String>,
    pub timestamp: topgun_core::Timestamp,
    pub call_timeout_ms: u64,
}

/// Typed operation variants dispatched through the pipeline.
#[derive(Debug)]
#[non_exhaustive]
pub enum Operation {
    /// Placeholder -- full variants will be added in G1-S2.
    _Placeholder(OperationContext),
}

/// Successful response from an operation handler.
#[derive(Debug)]
pub enum OperationResponse {
    Ack { call_id: u64 },
    Message(Box<topgun_core::messages::Message>),
    Messages(Vec<topgun_core::messages::Message>),
    NotImplemented { service_name: &'static str, call_id: u64 },
    Empty,
}

/// Errors returned by operation handlers.
#[derive(Debug, thiserror::Error)]
pub enum OperationError {
    #[error("unknown service: {name}")]
    UnknownService { name: String },
    #[error("operation timed out after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },
    #[error("server overloaded, try again later")]
    Overloaded,
    #[error("wrong service for operation")]
    WrongService,
    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

/// Errors from classifying a `Message` into an `Operation`.
#[derive(Debug, thiserror::Error)]
pub enum ClassifyError {
    #[error("server-to-client response cannot be classified as operation: {variant}")]
    ServerToClient { variant: &'static str },
    #[error("transport envelope must be unpacked before classification: {variant}")]
    TransportEnvelope { variant: &'static str },
    #[error("authentication message handled at transport layer: {variant}")]
    AuthMessage { variant: &'static str },
}
