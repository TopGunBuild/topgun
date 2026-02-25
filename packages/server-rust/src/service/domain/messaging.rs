//! Messaging domain service for topic-based pub/sub.
//!
//! Manages an in-memory `TopicRegistry` (topic name -> subscriber set) and
//! delivers published messages to subscribers via the `ConnectionRegistry`.
//! Topics are ephemeral: auto-removed when the last subscriber disconnects.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use dashmap::{DashMap, DashSet};
use tower::Service;

use topgun_core::messages::{Message, TopicMessageEventPayload};

use crate::network::connection::{ConnectionId, ConnectionRegistry, OutboundMessage};
use crate::service::operation::{
    service_names, Operation, OperationContext, OperationError, OperationResponse,
};
use crate::service::registry::{ManagedService, ServiceContext};

// ---------------------------------------------------------------------------
// TopicError
// ---------------------------------------------------------------------------

/// Errors specific to topic operations.
#[derive(Debug, thiserror::Error)]
pub enum TopicError {
    /// Topic name fails validation (empty, too long, or invalid characters).
    #[error("invalid topic name: {topic:?}")]
    InvalidTopicName { topic: String },
}

// ---------------------------------------------------------------------------
// TopicRegistry
// ---------------------------------------------------------------------------

/// Thread-safe in-memory registry mapping topic names to subscriber sets.
///
/// Uses `DashMap` + `DashSet` for lock-free concurrent access, consistent
/// with the `ConnectionRegistry` pattern used elsewhere in the server.
pub struct TopicRegistry {
    topics: DashMap<String, DashSet<ConnectionId>>,
}

impl TopicRegistry {
    /// Creates a new empty topic registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            topics: DashMap::new(),
        }
    }

    /// Validates a topic name against the allowed pattern `[\w\-.:/]+`.
    ///
    /// Rules: non-empty, max 256 characters, only alphanumeric, underscore,
    /// hyphen, dot, colon, and forward slash.
    fn validate_topic_name(topic: &str) -> Result<(), TopicError> {
        if topic.is_empty() || topic.len() > 256 {
            return Err(TopicError::InvalidTopicName {
                topic: topic.to_string(),
            });
        }
        let valid = topic.chars().all(|c| {
            c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == ':' || c == '/'
        });
        if !valid {
            return Err(TopicError::InvalidTopicName {
                topic: topic.to_string(),
            });
        }
        Ok(())
    }

    /// Adds a subscriber to a topic. Creates the topic entry if it does not exist.
    ///
    /// # Errors
    ///
    /// Returns `TopicError::InvalidTopicName` if the topic fails validation.
    pub fn subscribe(&self, topic: &str, conn_id: ConnectionId) -> Result<(), TopicError> {
        Self::validate_topic_name(topic)?;
        self.topics
            .entry(topic.to_string())
            .or_default()
            .insert(conn_id);
        Ok(())
    }

    /// Removes a subscriber from a topic. Removes the topic entry if the
    /// subscriber set becomes empty.
    pub fn unsubscribe(&self, topic: &str, conn_id: ConnectionId) {
        if let Some(subscribers) = self.topics.get(topic) {
            subscribers.remove(&conn_id);
            if subscribers.is_empty() {
                // Drop the read guard before removing to avoid deadlock.
                drop(subscribers);
                // Re-check emptiness under the write lock to avoid race.
                self.topics.remove_if(topic, |_, set| set.is_empty());
            }
        }
    }

    /// Removes a connection from all topics. Removes any topic entries that
    /// become empty as a result.
    pub fn unsubscribe_all(&self, conn_id: ConnectionId) {
        // Collect topic names first to avoid holding DashMap iterators during mutation.
        let topic_names: Vec<String> = self
            .topics
            .iter()
            .filter(|entry| entry.value().contains(&conn_id))
            .map(|entry| entry.key().clone())
            .collect();

        for topic in &topic_names {
            self.unsubscribe(topic, conn_id);
        }
    }

    /// Returns the current subscriber list for a topic (empty vec if the
    /// topic does not exist).
    #[must_use]
    pub fn subscribers(&self, topic: &str) -> Vec<ConnectionId> {
        self.topics
            .get(topic)
            .map(|set| set.iter().map(|id| *id).collect())
            .unwrap_or_default()
    }

    /// Returns the number of active topics (for testing).
    #[must_use]
    pub fn topic_count(&self) -> usize {
        self.topics.len()
    }
}

impl Default for TopicRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// MessagingService
// ---------------------------------------------------------------------------

/// Real messaging domain service handling topic subscribe, unsubscribe, and
/// publish operations. Replaces the `domain_stub!(MessagingService, ...)`
/// macro-generated stub.
pub struct MessagingService {
    topic_registry: Arc<TopicRegistry>,
    connection_registry: Arc<ConnectionRegistry>,
}

impl MessagingService {
    /// Creates a new `MessagingService` with its required dependencies.
    #[must_use]
    pub fn new(connection_registry: Arc<ConnectionRegistry>) -> Self {
        Self {
            topic_registry: Arc::new(TopicRegistry::new()),
            connection_registry,
        }
    }

    /// Returns a reference to the inner topic registry (for testing).
    #[must_use]
    pub fn topic_registry(&self) -> &TopicRegistry {
        &self.topic_registry
    }
}

// ---------------------------------------------------------------------------
// ManagedService implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl ManagedService for MessagingService {
    fn name(&self) -> &'static str {
        service_names::MESSAGING
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

// ---------------------------------------------------------------------------
// tower::Service<Operation> implementation
// ---------------------------------------------------------------------------

impl Service<Operation> for Arc<MessagingService> {
    type Response = OperationResponse;
    type Error = OperationError;
    type Future =
        Pin<Box<dyn Future<Output = Result<OperationResponse, OperationError>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, op: Operation) -> Self::Future {
        let svc = Arc::clone(self);
        Box::pin(async move {
            match op {
                Operation::TopicSubscribe { ctx, payload } => {
                    svc.handle_topic_subscribe(&ctx, &payload.topic).await
                }
                Operation::TopicUnsubscribe { ctx, payload } => {
                    svc.handle_topic_unsubscribe(&ctx, &payload.topic).await
                }
                Operation::TopicPublish { ctx, payload } => {
                    svc.handle_topic_publish(&ctx, &payload.topic, payload.data)
                }
                _ => Err(OperationError::WrongService),
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Handler implementations
// ---------------------------------------------------------------------------

impl MessagingService {
    /// Handles a `TopicSubscribe` operation: validates the topic name, adds the
    /// connection to the topic registry, and records the topic in the
    /// connection's metadata.
    async fn handle_topic_subscribe(
        &self,
        ctx: &OperationContext,
        topic: &str,
    ) -> Result<OperationResponse, OperationError> {
        let conn_id = ctx.connection_id.ok_or_else(|| {
            OperationError::Internal(anyhow::anyhow!(
                "TopicSubscribe requires a connection_id"
            ))
        })?;

        self.topic_registry
            .subscribe(topic, conn_id)
            .map_err(|e| OperationError::Internal(anyhow::anyhow!("{e}")))?;

        // Also track the topic in the connection's metadata.
        if let Some(handle) = self.connection_registry.get(conn_id) {
            let mut meta = handle.metadata.write().await;
            meta.topics.insert(topic.to_string());
        }

        Ok(OperationResponse::Empty)
    }

    /// Handles a `TopicUnsubscribe` operation: removes the connection from the
    /// topic registry and removes the topic from the connection's metadata.
    async fn handle_topic_unsubscribe(
        &self,
        ctx: &OperationContext,
        topic: &str,
    ) -> Result<OperationResponse, OperationError> {
        let conn_id = ctx.connection_id.ok_or_else(|| {
            OperationError::Internal(anyhow::anyhow!(
                "TopicUnsubscribe requires a connection_id"
            ))
        })?;

        self.topic_registry.unsubscribe(topic, conn_id);

        // Also remove the topic from the connection's metadata.
        if let Some(handle) = self.connection_registry.get(conn_id) {
            let mut meta = handle.metadata.write().await;
            meta.topics.remove(topic);
        }

        Ok(OperationResponse::Empty)
    }

    /// Handles a `TopicPublish` operation: validates the topic name, builds a
    /// `TopicMessageEventPayload`, serializes it as `Message::TopicMessage`,
    /// and sends it to each subscriber's connection (excluding the publisher).
    fn handle_topic_publish(
        &self,
        ctx: &OperationContext,
        topic: &str,
        data: rmpv::Value,
    ) -> Result<OperationResponse, OperationError> {
        let publisher_conn_id = ctx.connection_id.ok_or_else(|| {
            OperationError::Internal(anyhow::anyhow!(
                "TopicPublish requires a connection_id"
            ))
        })?;

        // Validate topic name (publish to nonexistent topic is OK, but name must be valid).
        TopicRegistry::validate_topic_name(topic)
            .map_err(|e| OperationError::Internal(anyhow::anyhow!("{e}")))?;

        let subscribers = self.topic_registry.subscribers(topic);
        if subscribers.is_empty() {
            return Ok(OperationResponse::Empty);
        }

        // Build the outgoing message.
        #[allow(clippy::cast_possible_truncation)]
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let event = Message::TopicMessage {
            payload: TopicMessageEventPayload {
                topic: topic.to_string(),
                data,
                publisher_id: ctx.client_id.clone(),
                timestamp,
            },
        };

        let bytes = rmp_serde::to_vec_named(&event)
            .map_err(|e| OperationError::Internal(anyhow::anyhow!("serialize error: {e}")))?;

        // Fan out to all subscribers except the publisher.
        for sub_conn_id in subscribers {
            if sub_conn_id == publisher_conn_id {
                continue;
            }
            if let Some(handle) = self.connection_registry.get(sub_conn_id) {
                // Best-effort delivery: skip full channels.
                let _ = handle.try_send(OutboundMessage::Binary(bytes.clone()));
            }
        }

        Ok(OperationResponse::Empty)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::SystemTime;

    use topgun_core::messages::{Message, TopicPubPayload, TopicSubPayload, TopicUnsubPayload};
    use topgun_core::Timestamp;
    use tower::ServiceExt;

    use super::*;
    use crate::network::config::ConnectionConfig;
    use crate::network::connection::{ConnectionKind, ConnectionRegistry, OutboundMessage};
    use crate::service::operation::{service_names, OperationContext};

    /// Helper: create a `MessagingService` with a shared `ConnectionRegistry`.
    fn make_service() -> (Arc<MessagingService>, Arc<ConnectionRegistry>) {
        let registry = Arc::new(ConnectionRegistry::new());
        let svc = Arc::new(MessagingService::new(Arc::clone(&registry)));
        (svc, registry)
    }

    fn make_timestamp() -> Timestamp {
        Timestamp {
            millis: 1_700_000_000_000,
            counter: 1,
            node_id: "test-node".to_string(),
        }
    }

    fn make_ctx(conn_id: Option<ConnectionId>) -> OperationContext {
        let mut ctx = OperationContext::new(1, service_names::MESSAGING, make_timestamp(), 5000);
        ctx.connection_id = conn_id;
        ctx
    }

    fn make_ctx_with_client(conn_id: Option<ConnectionId>, client_id: &str) -> OperationContext {
        let mut ctx = make_ctx(conn_id);
        ctx.client_id = Some(client_id.to_string());
        ctx
    }

    fn test_config() -> ConnectionConfig {
        ConnectionConfig::default()
    }

    // -- TopicRegistry unit tests --

    #[test]
    fn topic_registry_subscribe_and_subscribers() {
        let registry = TopicRegistry::new();
        let conn = ConnectionId(1);
        registry.subscribe("chat/room-1", conn).unwrap();
        let subs = registry.subscribers("chat/room-1");
        assert_eq!(subs, vec![conn]);
        assert_eq!(registry.topic_count(), 1);
    }

    #[test]
    fn topic_registry_unsubscribe_removes_subscriber() {
        let registry = TopicRegistry::new();
        let conn = ConnectionId(1);
        registry.subscribe("chat/room-1", conn).unwrap();
        registry.unsubscribe("chat/room-1", conn);
        assert!(registry.subscribers("chat/room-1").is_empty());
    }

    #[test]
    fn topic_registry_auto_removes_empty_topic() {
        let registry = TopicRegistry::new();
        let conn = ConnectionId(1);
        registry.subscribe("ephemeral", conn).unwrap();
        assert_eq!(registry.topic_count(), 1);
        registry.unsubscribe("ephemeral", conn);
        assert_eq!(registry.topic_count(), 0);
    }

    #[test]
    fn topic_registry_unsubscribe_all() {
        let registry = TopicRegistry::new();
        let conn = ConnectionId(1);
        let other = ConnectionId(2);
        registry.subscribe("topic-a", conn).unwrap();
        registry.subscribe("topic-b", conn).unwrap();
        registry.subscribe("topic-c", conn).unwrap();
        registry.subscribe("topic-a", other).unwrap();

        registry.unsubscribe_all(conn);

        assert!(registry.subscribers("topic-a").contains(&other));
        assert!(!registry.subscribers("topic-a").contains(&conn));
        assert!(registry.subscribers("topic-b").is_empty());
        assert!(registry.subscribers("topic-c").is_empty());
        // topic-a still exists because `other` is subscribed; topic-b and topic-c removed.
        assert_eq!(registry.topic_count(), 1);
    }

    #[test]
    fn topic_registry_nonexistent_topic_returns_empty() {
        let registry = TopicRegistry::new();
        assert!(registry.subscribers("nonexistent").is_empty());
    }

    #[test]
    fn topic_registry_invalid_name_empty() {
        let registry = TopicRegistry::new();
        let result = registry.subscribe("", ConnectionId(1));
        assert!(result.is_err());
    }

    #[test]
    fn topic_registry_invalid_name_special_chars() {
        let registry = TopicRegistry::new();
        let result = registry.subscribe("bad topic!", ConnectionId(1));
        assert!(result.is_err());
    }

    #[test]
    fn topic_registry_invalid_name_too_long() {
        let registry = TopicRegistry::new();
        let long_name = "a".repeat(257);
        let result = registry.subscribe(&long_name, ConnectionId(1));
        assert!(result.is_err());
    }

    #[test]
    fn topic_registry_valid_names() {
        let registry = TopicRegistry::new();
        // All these should be valid per the regex [\w\-.:/]+
        assert!(registry.subscribe("chat/room-1", ConnectionId(1)).is_ok());
        assert!(registry.subscribe("events:live", ConnectionId(1)).is_ok());
        assert!(registry.subscribe("my.topic.v2", ConnectionId(1)).is_ok());
        assert!(registry.subscribe("under_score", ConnectionId(1)).is_ok());
        assert!(registry.subscribe("a", ConnectionId(1)).is_ok());
        let max_name = "x".repeat(256);
        assert!(registry.subscribe(&max_name, ConnectionId(1)).is_ok());
    }

    // -- AC1: TopicSubscribe adds connection to topic registry and metadata --

    #[tokio::test]
    async fn ac1_topic_subscribe_adds_to_registry_and_metadata() {
        let (svc, registry) = make_service();
        let config = test_config();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &config);
        let conn_id = handle.id;

        let op = Operation::TopicSubscribe {
            ctx: make_ctx(Some(conn_id)),
            payload: TopicSubPayload {
                topic: "chat/room-1".to_string(),
            },
        };

        let resp = svc.clone().oneshot(op).await.unwrap();
        assert!(matches!(resp, OperationResponse::Empty));

        // Topic registry should contain the connection.
        assert!(svc.topic_registry().subscribers("chat/room-1").contains(&conn_id));

        // Connection metadata should contain the topic.
        let meta = handle.metadata.read().await;
        assert!(meta.topics.contains("chat/room-1"));
    }

    // -- AC2: TopicUnsubscribe removes connection from registry and metadata --

    #[tokio::test]
    async fn ac2_topic_unsubscribe_removes_from_registry_and_metadata() {
        let (svc, registry) = make_service();
        let config = test_config();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &config);
        let conn_id = handle.id;

        // First subscribe.
        let op = Operation::TopicSubscribe {
            ctx: make_ctx(Some(conn_id)),
            payload: TopicSubPayload {
                topic: "chat/room-1".to_string(),
            },
        };
        svc.clone().oneshot(op).await.unwrap();

        // Now unsubscribe.
        let op = Operation::TopicUnsubscribe {
            ctx: make_ctx(Some(conn_id)),
            payload: TopicUnsubPayload {
                topic: "chat/room-1".to_string(),
            },
        };
        let resp = svc.clone().oneshot(op).await.unwrap();
        assert!(matches!(resp, OperationResponse::Empty));

        // Topic registry should NOT contain the connection.
        assert!(!svc.topic_registry().subscribers("chat/room-1").contains(&conn_id));

        // Connection metadata should NOT contain the topic.
        let meta = handle.metadata.read().await;
        assert!(!meta.topics.contains("chat/room-1"));
    }

    // -- AC3: TopicPublish delivers to subscribers, excludes publisher --

    #[tokio::test]
    async fn ac3_topic_publish_delivers_to_subscribers_excludes_publisher() {
        let (svc, registry) = make_service();
        let config = test_config();

        let (handle_a, mut rx_a) = registry.register(ConnectionKind::Client, &config);
        let (handle_b, mut rx_b) = registry.register(ConnectionKind::Client, &config);
        let (handle_c, mut rx_c) = registry.register(ConnectionKind::Client, &config);

        let conn_a = handle_a.id;
        let conn_b = handle_b.id;
        let conn_c = handle_c.id;

        // Subscribe A, B, C to "news".
        for (svc_ref, cid) in [(Arc::clone(&svc), conn_a), (Arc::clone(&svc), conn_b), (Arc::clone(&svc), conn_c)] {
            let op = Operation::TopicSubscribe {
                ctx: make_ctx(Some(cid)),
                payload: TopicSubPayload {
                    topic: "news".to_string(),
                },
            };
            svc_ref.oneshot(op).await.unwrap();
        }

        // A publishes.
        let op = Operation::TopicPublish {
            ctx: make_ctx_with_client(Some(conn_a), "client-a"),
            payload: TopicPubPayload {
                topic: "news".to_string(),
                data: rmpv::Value::String("hello".into()),
            },
        };
        let resp = svc.oneshot(op).await.unwrap();
        assert!(matches!(resp, OperationResponse::Empty));

        // B should receive the message.
        let msg_b = rx_b.try_recv();
        assert!(msg_b.is_ok(), "B should have received a message");

        // C should receive the message.
        let msg_c = rx_c.try_recv();
        assert!(msg_c.is_ok(), "C should have received a message");

        // A should NOT receive the message (publisher excluded).
        let msg_a = rx_a.try_recv();
        assert!(msg_a.is_err(), "A (publisher) should NOT receive the message");

        // Verify the payload content of B's message.
        if let Ok(OutboundMessage::Binary(bytes)) = msg_b {
            let decoded: Message = rmp_serde::from_slice(&bytes).expect("deserialize");
            match decoded {
                Message::TopicMessage { payload } => {
                    assert_eq!(payload.topic, "news");
                    assert_eq!(payload.data, rmpv::Value::String("hello".into()));
                    assert_eq!(payload.publisher_id, Some("client-a".to_string()));
                    // Timestamp should be within 1 second of now.
                    #[allow(clippy::cast_possible_truncation)]
                    let now_ms = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64;
                    assert!(
                        now_ms.abs_diff(payload.timestamp) < 1000,
                        "timestamp {} should be within 1s of now {}",
                        payload.timestamp,
                        now_ms
                    );
                }
                other => panic!("expected TopicMessage, got {other:?}"),
            }
        } else {
            panic!("expected Binary message from B's channel");
        }
    }

    // -- AC4: TopicPublish to empty topic returns Empty --

    #[tokio::test]
    async fn ac4_publish_to_empty_topic_returns_empty() {
        let (svc, registry) = make_service();
        let config = test_config();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &config);

        let op = Operation::TopicPublish {
            ctx: make_ctx_with_client(Some(handle.id), "client-1"),
            payload: TopicPubPayload {
                topic: "empty-topic".to_string(),
                data: rmpv::Value::Nil,
            },
        };
        let resp = svc.oneshot(op).await.unwrap();
        assert!(matches!(resp, OperationResponse::Empty));
    }

    // -- AC5: Invalid topic name rejected --

    #[tokio::test]
    async fn ac5_invalid_topic_name_rejected_on_subscribe() {
        let (svc, registry) = make_service();
        let config = test_config();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &config);

        // Empty topic.
        let op = Operation::TopicSubscribe {
            ctx: make_ctx(Some(handle.id)),
            payload: TopicSubPayload {
                topic: String::new(),
            },
        };
        let result = svc.clone().oneshot(op).await;
        assert!(result.is_err());

        // Invalid chars.
        let op = Operation::TopicSubscribe {
            ctx: make_ctx(Some(handle.id)),
            payload: TopicSubPayload {
                topic: "bad topic!".to_string(),
            },
        };
        let result = svc.clone().oneshot(op).await;
        assert!(result.is_err());

        // Too long.
        let op = Operation::TopicSubscribe {
            ctx: make_ctx(Some(handle.id)),
            payload: TopicSubPayload {
                topic: "a".repeat(257),
            },
        };
        let result = svc.clone().oneshot(op).await;
        assert!(result.is_err());
    }

    // -- AC6: Topic auto-removed when last subscriber unsubscribes --

    #[tokio::test]
    async fn ac6_topic_auto_removed_when_last_subscriber_leaves() {
        let (svc, registry) = make_service();
        let config = test_config();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &config);
        let conn_id = handle.id;

        let op = Operation::TopicSubscribe {
            ctx: make_ctx(Some(conn_id)),
            payload: TopicSubPayload {
                topic: "ephemeral".to_string(),
            },
        };
        svc.clone().oneshot(op).await.unwrap();
        assert_eq!(svc.topic_registry().topic_count(), 1);

        let op = Operation::TopicUnsubscribe {
            ctx: make_ctx(Some(conn_id)),
            payload: TopicUnsubPayload {
                topic: "ephemeral".to_string(),
            },
        };
        svc.clone().oneshot(op).await.unwrap();
        assert_eq!(svc.topic_registry().topic_count(), 0);
    }

    // -- AC7: Missing connection_id on TopicSubscribe returns error --

    #[tokio::test]
    async fn ac7_missing_connection_id_on_subscribe_returns_error() {
        let (svc, _registry) = make_service();

        let op = Operation::TopicSubscribe {
            ctx: make_ctx(None),
            payload: TopicSubPayload {
                topic: "chat/room-1".to_string(),
            },
        };
        let result = svc.oneshot(op).await;
        assert!(
            matches!(result, Err(OperationError::Internal(_))),
            "expected Internal error, got {result:?}"
        );
    }

    // -- AC8: Wrong service returns WrongService error --

    #[tokio::test]
    async fn ac8_wrong_service_returns_error() {
        let (svc, _registry) = make_service();

        let op = Operation::GarbageCollect {
            ctx: make_ctx(None),
        };
        let result = svc.oneshot(op).await;
        assert!(
            matches!(result, Err(OperationError::WrongService)),
            "expected WrongService, got {result:?}"
        );
    }

    // -- AC9: ManagedService name is "messaging" --

    #[test]
    fn ac9_managed_service_name() {
        let registry = Arc::new(ConnectionRegistry::new());
        let svc = MessagingService::new(registry);
        assert_eq!(svc.name(), "messaging");
    }

    // -- AC11: unsubscribe_all removes connection from all topics --

    #[tokio::test]
    async fn ac11_unsubscribe_all_removes_connection_from_all_topics() {
        let (svc, registry) = make_service();
        let config = test_config();
        let (handle, _rx) = registry.register(ConnectionKind::Client, &config);
        let conn_id = handle.id;
        let (handle2, _rx2) = registry.register(ConnectionKind::Client, &config);
        let conn_id2 = handle2.id;

        // Subscribe conn to 3 topics, conn2 to topic-a.
        for topic in &["topic-a", "topic-b", "topic-c"] {
            svc.topic_registry().subscribe(topic, conn_id).unwrap();
        }
        svc.topic_registry().subscribe("topic-a", conn_id2).unwrap();

        assert_eq!(svc.topic_registry().topic_count(), 3);

        svc.topic_registry().unsubscribe_all(conn_id);

        // conn removed from all topics.
        assert!(!svc.topic_registry().subscribers("topic-a").contains(&conn_id));
        assert!(!svc.topic_registry().subscribers("topic-b").contains(&conn_id));
        assert!(!svc.topic_registry().subscribers("topic-c").contains(&conn_id));

        // conn2 still in topic-a.
        assert!(svc.topic_registry().subscribers("topic-a").contains(&conn_id2));

        // topic-b and topic-c auto-removed (only conn was subscribed).
        // topic-a remains because conn2 is still subscribed.
        assert_eq!(svc.topic_registry().topic_count(), 1);
    }

    // -- AC12: Missing connection_id on TopicPublish returns error --

    #[tokio::test]
    async fn ac12_missing_connection_id_on_publish_returns_error() {
        let (svc, _registry) = make_service();

        let op = Operation::TopicPublish {
            ctx: make_ctx(None),
            payload: TopicPubPayload {
                topic: "news".to_string(),
                data: rmpv::Value::Nil,
            },
        };
        let result = svc.oneshot(op).await;
        assert!(
            matches!(result, Err(OperationError::Internal(_))),
            "expected Internal error, got {result:?}"
        );
    }
}
