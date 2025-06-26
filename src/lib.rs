//! Custom rate limiting library with Redis and Axum support

mod backoff;
mod middleware;
mod redis_store;
mod types;

// Re-export key items for easier access
pub use middleware::{KeyExtractable, create_barnacle_layer, create_barnacle_layer_for_payload};
pub use redis_store::RedisBarnacleStore;
pub use tracing;
pub use types::{BarnacleConfig, BarnacleKey, BarnacleResult, ResetOnSuccess};

use async_trait::async_trait;

/// Trait to abstract the rate limiter storage backend (e.g., Redis)
#[async_trait]
pub trait BarnacleStore: Send + Sync {
    /// Increments the counter for the key and returns the current number of requests and remaining time until reset.
    async fn increment(&self, key: &BarnacleKey, config: &BarnacleConfig) -> BarnacleResult;
    /// Resets the counter for the key (e.g., after successful login).
    async fn reset(&self, key: &BarnacleKey) -> anyhow::Result<()>;
}
