//! Custom rate limiting library with Redis and Axum support

pub mod backoff;
pub mod middleware;
pub mod redis_store;
pub mod types;

// Re-export key items for easier access
pub use middleware::{KeyExtractable, create_barnacle_layer_for_payload};
pub use redis_store::RedisBarnacleStore;

use std::sync::Arc;

use async_trait::async_trait;
use types::{BarnacleConfig, BarnacleKey, BarnacleResult};

/// Trait to abstract the rate limiter storage backend (e.g., Redis)
#[async_trait]
pub trait BarnacleStore: Send + Sync {
    /// Increments the counter for the key and returns the current number of requests and remaining time until reset.
    async fn increment(&self, key: &BarnacleKey, config: &BarnacleConfig) -> BarnacleResult;
    /// Resets the counter for the key (e.g., after successful login).
    async fn reset(&self, key: &BarnacleKey) -> anyhow::Result<()>;
}

/// Utility function to create the rate limiting middleware
pub fn barnacle_layer<S: BarnacleStore + 'static>(
    store: Arc<S>,
    config: BarnacleConfig,
) -> middleware::BarnacleLayer<(), S> {
    middleware::BarnacleLayer::new(store, config)
}
