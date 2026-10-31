use crate::types::Principal;

/// Per-request context carrying identity and tracing information.
/// Threaded through server operations for auth and audit; a caller's tenant
/// lives in [`Principal`] only.
#[derive(Debug, Clone)]
pub struct RequestContext {
    /// Identifier of the server node handling this request.
    pub node_id: String,
    /// Authenticated principal, if the request is authenticated.
    pub principal: Option<Principal>,
    /// Distributed trace identifier for observability.
    pub trace_id: String,
}
