use crate::types::Principal;

/// Per-request context carrying identity, tenancy, and tracing information.
/// Threaded through all server operations for auth, audit, and multi-tenant isolation.
#[derive(Debug, Clone)]
pub struct RequestContext {
    /// Identifier of the server node handling this request.
    pub node_id: String,
    /// Tenant scope for multi-tenant isolation. `None` for single-tenant deployments.
    pub tenant_id: Option<String>,
    /// Authenticated principal, if the request is authenticated.
    pub principal: Option<Principal>,
    /// Distributed trace identifier for observability.
    pub trace_id: String,
}
