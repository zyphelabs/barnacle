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
//! use barnacle_rs::{
//!     create_api_key_layer, ApiKeyMiddlewareConfig, BarnacleConfig
//! };
//! #[cfg(feature = "redis")]
//! use barnacle_rs::{RedisApiKeyStore, RedisBarnacleStore, deadpool_redis};
//! use std::time::Duration;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create Redis stores (requires "redis" feature)
//! #[cfg(feature = "redis")]
//! let redis_pool = deadpool_redis::Config::from_url("redis://localhost")
//!     .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
//!
//! #[cfg(feature = "redis")]
//! let api_key_store = RedisApiKeyStore::new(redis_pool.clone());
//! #[cfg(feature = "redis")]
//! let rate_limit_store = RedisBarnacleStore::new(redis_pool);
//!
//! // Create middleware
//! #[cfg(feature = "redis")]
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
mod error;
mod middleware;
mod redis_store;
mod types;

// Re-export key items for easier access
pub use api_key_middleware::{
    create_api_key_layer, create_api_key_layer_with_config,
    create_api_key_layer_with_custom_validator, ApiKeyLayer,
};
pub use api_key_store::{ApiKeyStore, StaticApiKeyStore};
pub use error::{BarnacleError, BarnacleResult};
pub use middleware::{
    create_barnacle_layer, create_barnacle_layer_for_payload, BarnacleLayer, KeyExtractable,
};
pub use tracing;
pub use types::{
    ApiKeyExtractionResult, ApiKeyMiddlewareConfig, ApiKeyValidationResult, BarnacleConfig,
    BarnacleContext, BarnacleKey, ResetOnSuccess, StaticApiKeyConfig,
};

// Re-export the legacy BarnacleResult type from types.rs for backward compatibility
pub use types::BarnacleResult as LegacyBarnacleResult;

// Redis-specific exports (only available with "redis" feature)
#[cfg(feature = "redis")]
pub use api_key_store::RedisApiKeyStore;
#[cfg(feature = "redis")]
pub use redis_store::RedisBarnacleStore;

// Re-export commonly used external dependencies (only with redis feature)
#[cfg(feature = "redis")]
pub use deadpool_redis;

use async_trait::async_trait;

pub const BARNACLE_EMAIL_KEY_PREFIX: &str = "barnacle:email";
pub const BARNACLE_API_KEY_PREFIX: &str = "barnacle:api_keys";
pub const BARNACLE_IP_PREFIX: &str = "barnacle:ip";
pub const BARNACLE_CUSTOM_PREFIX: &str = "barnacle:custom";

/// Trait to abstract the rate limiter storage backend (e.g., Redis)
#[async_trait]
pub trait BarnacleStore: Send + Sync {
    /// Increments the counter for the key and returns the current number of requests and remaining time until reset.
    async fn increment(
        &self,
        context: &BarnacleContext,
        config: &BarnacleConfig,
    ) -> Result<types::BarnacleResult, BarnacleError>;
    /// Resets the counter for the key (e.g., after successful login).
    async fn reset(&self, context: &BarnacleContext) -> Result<(), BarnacleError>;
}
