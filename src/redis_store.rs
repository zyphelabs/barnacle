#[cfg(feature = "redis")]
use std::sync::Arc;
#[cfg(feature = "redis")]
use std::time::Duration;

#[cfg(feature = "redis")]
use async_trait::async_trait;
#[cfg(feature = "redis")]
use deadpool_redis::redis::AsyncCommands;
#[cfg(feature = "redis")]
use deadpool_redis::{Connection, Pool};

use crate::{
    types::{BarnacleConfig, BarnacleContext, BarnacleKey, BarnacleResult},
    BarnacleStore,
};

#[cfg(feature = "redis")]
struct RedisBarnacleStoreInner {
    pool: Pool,
}

#[cfg(feature = "redis")]
impl RedisBarnacleStoreInner {
    fn new(pool: Pool) -> Self {
        Self { pool }
    }

    async fn get_connection(&self) -> Result<Connection, deadpool_redis::PoolError> {
        self.pool.get().await
    }

    fn get_redis_key(&self, context: &BarnacleContext) -> String {
        let base_key = match &context.key {
            BarnacleKey::Email(email) => format!("barnacle:email:{}", email),
            BarnacleKey::ApiKey(api_key) => format!("barnacle:api_key:{}", api_key),
            BarnacleKey::Ip(ip) => format!("barnacle:ip:{}", ip),
            BarnacleKey::Custom(custom_data) => format!("barnacle:custom:{}", custom_data),
        };

        // Include path and method in the Redis key
        format!("{}:{}:{}", base_key, context.method, context.path)
    }
}

/// Implementation of BarnacleStore using Redis with connection pooling.
/// This struct encapsulates Arc internally, so consumers don't need to wrap it.
#[cfg(feature = "redis")]
#[derive(Clone)]
pub struct RedisBarnacleStore {
    inner: Arc<RedisBarnacleStoreInner>,
}

#[cfg(feature = "redis")]
impl RedisBarnacleStore {
    /// Create a new Redis store with connection pooling
    pub fn new(pool: Pool) -> Self {
        Self {
            inner: Arc::new(RedisBarnacleStoreInner::new(pool)),
        }
    }

    /// Create a new Redis store from a Redis URL
    pub async fn from_url(url: &str) -> Result<Self, deadpool_redis::PoolError> {
        let cfg = deadpool_redis::Config::from_url(url);
        let pool = cfg
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .map_err(|e| {
                deadpool_redis::PoolError::Backend(deadpool_redis::redis::RedisError::from(
                    std::io::Error::new(std::io::ErrorKind::Other, e),
                ))
            })?;
        Ok(Self::new(pool))
    }

    /// Create a new Redis store with custom pool configuration
    pub fn with_pool_config(url: &str, max_size: usize) -> Result<Self, deadpool_redis::PoolError> {
        let mut cfg = deadpool_redis::Config::from_url(url);
        cfg.pool = Some(deadpool_redis::PoolConfig {
            max_size,
            ..Default::default()
        });
        let pool = cfg
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .map_err(|e| {
                deadpool_redis::PoolError::Backend(deadpool_redis::redis::RedisError::from(
                    std::io::Error::new(std::io::ErrorKind::Other, e),
                ))
            })?;
        Ok(Self::new(pool))
    }
}

#[cfg(feature = "redis")]
#[async_trait]
impl BarnacleStore for RedisBarnacleStore {
    async fn increment(
        &self,
        context: &BarnacleContext,
        config: &BarnacleConfig,
    ) -> BarnacleResult {
        let redis_key = self.inner.get_redis_key(context);
        let window_seconds = config.window.as_secs() as usize;

        // Get Redis connection from pool
        let mut conn = match self.inner.get_connection().await {
            Ok(conn) => conn,
            Err(e) => {
                // If Redis pool is exhausted or unavailable, log the error and deny the request for safety
                tracing::error!("Redis connection pool error: {}", e);
                return BarnacleResult {
                    allowed: false,
                    remaining: 0,
                    retry_after: Some(config.window),
                };
            }
        };

        // Get current count and TTL using individual commands
        let current_count: Option<u32> = match conn.get(&redis_key).await {
            Ok(count) => count,
            Err(e) => {
                tracing::error!("Redis get operation failed: {}", e);
                return BarnacleResult {
                    allowed: false,
                    remaining: 0,
                    retry_after: Some(config.window),
                };
            }
        };

        let ttl: i32 = match conn.ttl(&redis_key).await {
            Ok(ttl) => ttl,
            Err(e) => {
                tracing::error!("Redis TTL operation failed: {}", e);
                return BarnacleResult {
                    allowed: false,
                    remaining: 0,
                    retry_after: Some(config.window),
                };
            }
        };

        let current_count = current_count.unwrap_or(0);
        let ttl = ttl.max(0) as u32;

        // Check if we're within the rate limit
        if current_count >= config.max_requests {
            // Rate limit exceeded
            let retry_after = if ttl > 0 {
                Duration::from_secs(ttl as u64)
            } else {
                config.window
            };

            return BarnacleResult {
                allowed: false,
                remaining: 0,
                retry_after: Some(retry_after),
            };
        }

        // Increment the counter
        let new_count: u32 = match conn.incr(&redis_key, 1).await {
            Ok(count) => count,
            Err(e) => {
                // If increment fails, log the error and deny the request for safety
                tracing::error!("Redis increment operation failed: {}", e);
                return BarnacleResult {
                    allowed: false,
                    remaining: 0,
                    retry_after: Some(config.window),
                };
            }
        };

        // Set expiration if this is the first increment
        if new_count == 1 {
            let _: Result<(), _> = conn.expire(&redis_key, window_seconds as i64).await;
        }

        BarnacleResult {
            allowed: true,
            remaining: config.max_requests.saturating_sub(new_count),
            retry_after: None,
        }
    }

    async fn reset(&self, context: &BarnacleContext) -> anyhow::Result<()> {
        let redis_key = self.inner.get_redis_key(context);

        let mut conn = self.inner.get_connection().await?;
        let _: () = conn.del(&redis_key).await?;

        Ok(())
    }
}
