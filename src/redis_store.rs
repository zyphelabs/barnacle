use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use redis::AsyncCommands;

use crate::{
    BarnacleStore,
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
        }
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
            Err(_) => {
                // If Redis is unavailable, allow the request but log the error
                return BarnacleResult {
                    allowed: true,
                    remaining: config.max_requests,
                    retry_after: None,
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
            Err(_) => {
                // If Redis operation fails, allow the request
                return BarnacleResult {
                    allowed: true,
                    remaining: config.max_requests,
                    retry_after: None,
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
            Err(_) => {
                // If increment fails, allow the request
                return BarnacleResult {
                    allowed: true,
                    remaining: config.max_requests - current_count,
                    retry_after: None,
                };
            }
        };

        // Set expiration if this is the first increment
        if new_count == 1 {
            let _: Result<(), _> = conn.expire(&redis_key, window_seconds as i64).await;
        }

        BarnacleResult {
            allowed: true,
            remaining: config.max_requests - new_count,
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
