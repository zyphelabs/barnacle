use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use redis::AsyncCommands;

use crate::{
    BarnacleStore,
    backoff::next_backoff,
    types::{BarnacleConfig, BarnacleKey, BarnacleResult},
};

/// Implementation of BarnacleStore using Redis
pub struct RedisBarnacleStore {
    pub client: Arc<redis::Client>,
}

impl RedisBarnacleStore {
    pub fn new(client: Arc<redis::Client>) -> Self {
        Self { client }
    }

    fn get_redis_key(&self, key: &BarnacleKey) -> String {
        match key {
            BarnacleKey::Email(email) => format!("barnacle:email:{}", email),
            BarnacleKey::ApiKey(api_key) => format!("barnacle:api_key:{}", api_key),
            BarnacleKey::Ip(ip) => format!("barnacle:ip:{}", ip),
            BarnacleKey::Custom(custom_data) => format!("barnacle:custom:{}", custom_data),
        }
    }

    fn get_failure_count_key(&self, key: &BarnacleKey) -> String {
        match key {
            BarnacleKey::Email(email) => format!("barnacle:failures:email:{}", email),
            BarnacleKey::ApiKey(api_key) => format!("barnacle:failures:api_key:{}", api_key),
            BarnacleKey::Ip(ip) => format!("barnacle:failures:ip:{}", ip),
            BarnacleKey::Custom(custom_data) => format!("barnacle:failures:custom:{}", custom_data),
        }
    }

    /// Increment failure count for backoff calculation
    async fn increment_failure_count(&self, key: &BarnacleKey) -> Result<u32, redis::RedisError> {
        let failure_key = self.get_failure_count_key(key);
        let mut conn = self.client.get_async_connection().await?;

        let count: u32 = conn.incr(&failure_key, 1).await?;
        // Set expiration to prevent indefinite storage (24 hours)
        let _: () = conn.expire(&failure_key, 86400).await?;

        Ok(count)
    }

    /// Get current failure count
    async fn get_failure_count(&self, key: &BarnacleKey) -> u32 {
        let failure_key = self.get_failure_count_key(key);
        let mut conn = match self.client.get_async_connection().await {
            Ok(conn) => conn,
            Err(_) => return 0,
        };

        conn.get(&failure_key).await.unwrap_or(0)
    }
}

#[async_trait]
impl BarnacleStore for RedisBarnacleStore {
    async fn increment(&self, key: &BarnacleKey, config: &BarnacleConfig) -> BarnacleResult {
        let redis_key = self.get_redis_key(key);
        let window_seconds = config.window.as_secs() as usize;

        // Get Redis connection
        let mut conn = match self.client.get_async_connection().await {
            Ok(conn) => conn,
            Err(e) => {
                // If Redis is unavailable, log the error and deny the request for safety
                eprintln!("Redis connection failed: {}", e);
                return BarnacleResult {
                    allowed: false,
                    remaining: 0,
                    retry_after: Some(config.window),
                };
            }
        };

        // Use Redis MULTI/EXEC for atomic operations
        let (current_count, ttl): (Option<u32>, Option<u32>) = match redis::pipe()
            .atomic()
            .get(&redis_key)
            .ttl(&redis_key)
            .query_async(&mut conn)
            .await
        {
            Ok(result) => result,
            Err(e) => {
                // If Redis operation fails, log the error and deny the request for safety
                eprintln!("Redis pipeline operation failed: {}", e);
                return BarnacleResult {
                    allowed: false,
                    remaining: 0,
                    retry_after: Some(config.window),
                };
            }
        };

        let current_count = current_count.unwrap_or(0);
        let ttl = ttl.unwrap_or(0);

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
                eprintln!("Redis increment operation failed: {}", e);
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

    async fn reset(&self, key: &BarnacleKey) -> anyhow::Result<()> {
        let redis_key = self.get_redis_key(key);

        let mut conn = self.client.get_async_connection().await?;
        let _: () = conn.del(&redis_key).await?;

        Ok(())
    }
}
