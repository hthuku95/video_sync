use futures::StreamExt;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use std::time::Duration;
use tokio::sync::mpsc;

/// A Redis pub/sub bus that bridges Fargate instances.
///
/// Channels follow the naming convention:
/// - `feedback:{session_id}` — user follow-up messages → running agent
/// - `progress:{session_id}` — agent progress updates → WebSocket
/// - `control:{job_id}` — Pause/Resume/Cancel → running job
///
/// Subscribe opens a dedicated Redis connection and spawns a background task
/// that forwards messages into a local mpsc channel. When the returned
/// `UnboundedReceiver` is dropped, the background task exits and the Redis
/// subscription is cleaned up.
#[derive(Clone)]
pub struct PubSubBus {
    /// Connection manager used for publishing (shareable across tasks).
    conn_mgr: ConnectionManager,
    /// Redis URL string, kept here to open fresh connections for subscriptions.
    redis_url: String,
}

impl PubSubBus {
    /// Connect to Redis and return a PubSubBus.
    ///
    /// Returns `Err` immediately if no URL is provided — no default fallback
    /// to avoid hanging when Redis is not configured.
    pub async fn connect(redis_url: Option<&str>) -> Result<Self, String> {
        let url = redis_url
            .filter(|u| !u.is_empty())
            .ok_or_else(|| "REDIS_URL not set — Redis pub/sub disabled".to_string())?;
        let client = redis::Client::open(url).map_err(|e| format!("Redis URL error: {}", e))?;
        let conn_mgr = tokio::time::timeout(
            Duration::from_secs(5),
            ConnectionManager::new(client),
        )
        .await
        .map_err(|_| "Redis connection timeout after 5s".to_string())?
        .map_err(|e| format!("Redis connect error: {}", e))?;
        tracing::info!("✅ Redis PubSubBus connected to {}", url);
        Ok(Self {
            conn_mgr,
            redis_url: url.to_string(),
        })
    }

    /// Publish a string payload to a channel.
    /// Returns the number of subscribers that received the message.
    pub async fn publish(&self, channel: &str, payload: &str) -> Result<i64, String> {
        let mut conn = self.conn_mgr.clone();
        conn.publish(channel, payload)
            .await
            .map_err(|e| format!("Redis publish error: {}", e))
    }

    /// Subscribe to a channel and get a local `mpsc::UnboundedReceiver`.
    ///
    /// Opens a fresh Redis connection for the subscription. The background
    /// task exits when the returned receiver is dropped.
    pub async fn subscribe(&self, channel: &str) -> Result<mpsc::UnboundedReceiver<String>, String> {
        let client =
            redis::Client::open(self.redis_url.as_str())
                .map_err(|e| format!("Redis client error: {}", e))?;
        let conn = tokio::time::timeout(
            Duration::from_secs(5),
            client.get_async_connection(),
        )
        .await
        .map_err(|_| "Redis subscribe connection timeout after 5s".to_string())?
        .map_err(|e| format!("Redis conn error: {}", e))?;
        let mut pubsub = conn.into_pubsub();
        pubsub
            .subscribe(channel)
            .await
            .map_err(|e| format!("Redis subscribe error: {}", e))?;

        let (tx, rx) = mpsc::unbounded_channel::<String>();
        let chan_name = channel.to_string();

        tokio::spawn(async move {
            let mut stream = pubsub.into_on_message();
            while let Some(msg) = stream.next().await {
                let payload: String = msg.get_payload().unwrap_or_default();
                if tx.send(payload).is_err() {
                    break; // Receiver dropped
                }
            }
            tracing::info!("🗑️ Redis pubsub subscription dropped: {}", chan_name);
        });

        Ok(rx)
    }
}
