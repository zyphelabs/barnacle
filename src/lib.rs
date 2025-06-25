//! Custom rate limiting library with Redis and Axum support

pub mod backoff;
pub mod key_extractor;
pub mod middleware;
pub mod redis_store;
pub mod types;

// Re-export key items for easier access
pub use key_extractor::{KeyExtractable, create_generic_rate_limit_layer};
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
) -> middleware::BarnacleLayer<S> {
    // Default extractor: always None, so fallback to IP
    let extractor: std::sync::Arc<
        dyn Fn(&axum::http::Request<axum::body::Body>) -> Option<BarnacleKey> + Send + Sync,
    > = std::sync::Arc::new(|_req| None);
    middleware::BarnacleLayer::new(store, config, extractor)
}
