//! Custom rate limiting library with Redis and Axum support
//!
//! This library provides middleware for API rate limiting and API key validation.
//!
//! ## Features
//!
//! - **Rate Limiting**: Configurable rate limiting with Redis backend
//! - **API Key Validation**: Validate requests using x-api-key header
//! - **Per-Key Rate Limits**: Different rate limits per API key
//! - **Extensible Design**: Custom key stores and rate limiting strategies
//! - **Redis Integration**: Default Redis-based storage for keys and rate limits
//! - **Axum Middleware**: Ready-to-use middleware for Axum web framework
//!
//! ## Basic Usage
//!
//! ```rust,no_run
//! use barnacle::{
//!     create_api_key_layer, RedisApiKeyStore, RedisBarnacleStore,
//!     ApiKeyMiddlewareConfig, BarnacleConfig
//! };
//! use std::sync::Arc;
//! use std::time::Duration;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create Redis stores
//! let redis_pool = deadpool_redis::Config::from_url("redis://localhost")
//!     .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
//!
//! let api_key_store = Arc::new(RedisApiKeyStore::new(
//!     redis_pool.clone(),
//!     BarnacleConfig::default()
//! ));
//! let rate_limit_store = Arc::new(RedisBarnacleStore::new(redis_pool));
//!
//! // Create middleware
//! let middleware = create_api_key_layer(api_key_store, rate_limit_store);
//!
//! // Use with Axum router
//! // let app = axum::Router::new()
//! //     .route("/api/data", axum::routing::get(handler))
//! //     .layer(middleware);
//! # Ok(())
//! # }
//! ```

mod api_key_middleware;
mod api_key_store;
mod middleware;
mod redis_store;
mod types;

// Re-export key items for easier access
pub use api_key_middleware::{ApiKeyLayer, create_api_key_layer, create_api_key_layer_with_config};
pub use api_key_store::{ApiKeyStore, InMemoryApiKeyStore, RedisApiKeyStore, StaticApiKeyStore};
pub use middleware::{KeyExtractable, create_barnacle_layer, create_barnacle_layer_for_payload};
pub use redis_store::RedisBarnacleStore;
pub use tracing;
pub use types::{
    ApiKeyMiddlewareConfig, ApiKeyValidationResult, BarnacleConfig, BarnacleKey, BarnacleResult,
    ResetOnSuccess, StaticApiKeyConfig,
};

use async_trait::async_trait;

/// Trait to abstract the rate limiter storage backend (e.g., Redis)
#[async_trait]
pub trait BarnacleStore: Send + Sync {
    /// Increments the counter for the key and returns the current number of requests and remaining time until reset.
    async fn increment(&self, key: &BarnacleKey, config: &BarnacleConfig) -> BarnacleResult;
    /// Resets the counter for the key (e.g., after successful login).
    async fn reset(&self, key: &BarnacleKey) -> anyhow::Result<()>;
}
