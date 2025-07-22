use async_trait::async_trait;
#[cfg(feature = "redis")]
use deadpool_redis::redis::AsyncCommands;
#[cfg(feature = "redis")]
use deadpool_redis::{Connection, Pool};

use crate::error::BarnacleError;
use crate::types::{ApiKeyValidationResult, BarnacleConfig, StaticApiKeyConfig};

/// Trait for API key validation and configuration retrieval
#[async_trait]
pub trait ApiKeyStore<T = String>: Send + Sync {
    /// Validate an API key and return its configuration
    async fn validate_key(&self, api_key: &str)
        -> Result<ApiKeyValidationResult<T>, BarnacleError>;

    /// Optional: Get rate limit configuration for a specific key
    /// This allows for dynamic per-key configuration
    async fn get_rate_limit_config(&self, api_key: &str) -> Option<BarnacleConfig> {
        // Default implementation returns None, letting validate_key handle config
        let _ = api_key;
        None
    }

    /// Optional: Cache a validated API key for future requests
    /// Default implementation does nothing - stores can override if they support caching
    async fn try_cache_key(
        &self,
        api_key: &str,
        config: &BarnacleConfig,
        ttl_seconds: Option<u64>,
    ) -> Result<(), BarnacleError> {
        let _ = (api_key, config, ttl_seconds);
        Ok(()) // Default: do nothing
    }
}

#[cfg(feature = "redis")]
#[derive(Clone)]
pub struct RedisApiKeyStore {
    pool: Pool,
    default_config: BarnacleConfig,
    key_prefix: String,
}

#[cfg(feature = "redis")]
impl RedisApiKeyStore {
    pub fn new(pool: Pool) -> Self {
        Self {
            pool,
            default_config: BarnacleConfig::default(),
            key_prefix: "barnacle:api_keys".to_string(),
        }
    }

    pub fn new_with_config(pool: Pool, config: BarnacleConfig) -> Self {
        Self {
            pool,
            default_config: config,
            key_prefix: "barnacle:api_keys".to_string(),
        }
    }

    pub fn from_url(url: &str) -> Result<Self, deadpool_redis::PoolError> {
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

    pub fn with_key_prefix(mut self, prefix: String) -> Self {
        self.key_prefix = prefix;
        self
    }

    async fn get_connection(&self) -> Result<Connection, deadpool_redis::PoolError> {
        self.pool.get().await
    }

    fn get_redis_key(&self, api_key: &str) -> String {
        format!("{}:{}", self.key_prefix, api_key)
    }

    fn get_config_key(&self, api_key: &str) -> String {
        format!("{}:config:{}", self.key_prefix, api_key)
    }

    pub async fn save_key(
        &self,
        api_key: &str,
        config: Option<&BarnacleConfig>,
        ttl_seconds: Option<u64>,
    ) -> Result<(), BarnacleError> {
        let redis_key = self.get_redis_key(api_key);
        let config_key = self.get_config_key(api_key);
        let default_ttl: u64 = 24 * 60 * 60; // 24 hours
        let ttl_api_key_secs: u64 = ttl_seconds.unwrap_or(default_ttl);

        tracing::debug!("Saving API key: {}", api_key);

        let mut conn = self.get_connection().await.map_err(|e| {
            BarnacleError::connection_pool_error("Failed to get Redis connection", Box::new(e))
        })?;

        conn.set_ex::<_, _, ()>(&redis_key, 1, ttl_api_key_secs)
            .await
            .map_err(|e| {
                BarnacleError::store_error_with_source("Failed to save API key", Box::new(e))
            })?;

        if let Some(cfg) = config {
            let config_json = serde_json::to_string(cfg)
                .map_err(|e| BarnacleError::json_error("Failed to serialize config", e))?;
            conn.set_ex::<_, _, ()>(&config_key, config_json, ttl_api_key_secs)
                .await
                .map_err(|e| {
                    BarnacleError::store_error_with_source(
                        "Failed to save API key config",
                        Box::new(e),
                    )
                })?;
        }

        Ok(())
    }

    /// Validates an API key with a fallback mechanism:
    /// 1. First checks if the key exists in Redis
    /// 2. If not, calls the provided validator function
    /// 3. If the validator returns a valid result, saves the key to Redis
    ///
    /// This is useful for validating API keys against a database only when needed
    pub async fn validate_key_with_fallback<F, Fut, E>(
        &self,
        api_key: &str,
        validator: F,
        config: Option<&BarnacleConfig>,
        ttl_seconds: Option<u64>,
    ) -> Result<ApiKeyValidationResult<String>, E>
    where
        F: FnOnce(String) -> Fut,
        Fut: std::future::Future<Output = Result<Option<String>, E>>,
        E: std::fmt::Debug,
    {
        // First try Redis
        let validation_result = self.validate_key(api_key).await;

        if let Ok(result) = validation_result {
            if result.valid {
                tracing::debug!("API key found in Redis cache: {}", api_key);
                return Ok(result);
            }
        }

        // If not in Redis, validate with the provided function
        tracing::debug!(
            "API key not found in Redis, validating externally: {}",
            api_key
        );

        match validator(api_key.to_string()).await {
            Ok(Some(key_id)) => {
                tracing::debug!("API key validated successfully: {}", api_key);

                // Save to Redis for future use
                let rate_limit_config = config
                    .cloned()
                    .unwrap_or_else(|| self.default_config.clone());

                if let Err(e) = self
                    .save_key(api_key, Some(&rate_limit_config), ttl_seconds)
                    .await
                {
                    tracing::warn!("Failed to cache API key in Redis: {}", e);
                    // Continue even if caching fails
                }

                Ok(ApiKeyValidationResult::valid_with_config(
                    key_id,
                    rate_limit_config,
                ))
            }
            Ok(None) => {
                tracing::warn!("API key validation failed: {}", api_key);
                Ok(ApiKeyValidationResult::invalid())
            }
            Err(e) => {
                tracing::error!("API key validation error: {:?}", e);
                Ok(ApiKeyValidationResult::invalid())
            }
        }
    }

    /// Invalidates all API keys from the Redis cache
    /// This is useful when API keys are modified in the database
    pub async fn invalidate_all_keys(&self) -> Result<u32, BarnacleError> {
        tracing::trace!("Invalidating all API keys from Redis cache");

        let mut conn = self.get_connection().await.map_err(|e| {
            BarnacleError::connection_pool_error("Failed to get Redis connection", Box::new(e))
        })?;

        // Find all keys matching our prefix pattern
        let pattern = format!("{}:*", self.key_prefix);
        let keys: Vec<String> = conn.keys(&pattern).await.map_err(|e| {
            BarnacleError::store_error_with_source("Failed to get keys pattern", Box::new(e))
        })?;

        if keys.is_empty() {
            tracing::debug!("No API keys found to invalidate");
            return Ok(0);
        }

        // Delete all found keys
        let deleted_count: u32 = conn.del(&keys).await.map_err(|e| {
            BarnacleError::store_error_with_source("Failed to delete keys", Box::new(e))
        })?;

        tracing::debug!("Invalidated {} API key cache entries", deleted_count);
        Ok(deleted_count)
    }
}

#[cfg(feature = "redis")]
#[async_trait]
impl ApiKeyStore<String> for RedisApiKeyStore {
    async fn validate_key(
        &self,
        api_key: &str,
    ) -> Result<ApiKeyValidationResult<String>, BarnacleError> {
        let redis_key = self.get_redis_key(api_key);
        let config_key = self.get_config_key(api_key);

        tracing::debug!("Validating API key: {}", api_key);

        let mut conn = match self.get_connection().await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::error!("Redis connection error during API key validation: {}", e);
                return Err(BarnacleError::connection_pool_error(
                    "Redis connection error",
                    Box::new(e),
                ));
            }
        };

        // Check if the API key exists
        let key_exists: bool = match conn.exists(&redis_key).await {
            Ok(exists) => exists,
            Err(e) => {
                tracing::error!("Redis EXISTS operation failed for API key: {}", e);
                return Err(BarnacleError::store_error_with_source(
                    "Redis EXISTS operation failed",
                    Box::new(e),
                ));
            }
        };

        if !key_exists {
            tracing::debug!("API key not found: {}", api_key);
            return Ok(ApiKeyValidationResult::invalid());
        }

        // Try to get custom configuration for this key
        let config: Option<String> = match conn.get(&config_key).await {
            Ok(config) => config,
            Err(e) => {
                tracing::warn!("Failed to get config for API key, using default: {}", e);
                None
            }
        };

        let rate_limit_config = if let Some(config_json) = config {
            // Parse the JSON configuration
            match serde_json::from_str::<BarnacleConfig>(&config_json) {
                Ok(config) => config,
                Err(e) => {
                    tracing::warn!("Failed to parse config for API key, using default: {}", e);
                    self.default_config.clone()
                }
            }
        } else {
            self.default_config.clone()
        };

        Ok(ApiKeyValidationResult::valid_with_config(
            api_key.to_string(),
            rate_limit_config,
        ))
    }

    async fn get_rate_limit_config(&self, api_key: &str) -> Option<BarnacleConfig> {
        let config_key = self.get_config_key(api_key);

        let mut conn = match self.get_connection().await {
            Ok(conn) => conn,
            Err(_) => return None,
        };

        let config: Option<String> = conn.get(&config_key).await.ok().flatten();

        config.and_then(|config_json| serde_json::from_str::<BarnacleConfig>(&config_json).ok())
    }

    async fn try_cache_key(
        &self,
        api_key: &str,
        config: &BarnacleConfig,
        ttl_seconds: Option<u64>,
    ) -> Result<(), BarnacleError> {
        self.save_key(api_key, Some(config), ttl_seconds).await
    }
}

/// Static API key store that uses a predefined set of keys
/// Useful for simple configurations where keys are known at compile time
pub struct StaticApiKeyStore {
    config: StaticApiKeyConfig,
}

impl StaticApiKeyStore {
    pub fn new(config: StaticApiKeyConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl ApiKeyStore<String> for StaticApiKeyStore {
    async fn validate_key(
        &self,
        api_key: &str,
    ) -> Result<ApiKeyValidationResult<String>, BarnacleError> {
        if self.config.key_configs.contains_key(api_key) {
            let config = self.config.get_config_for_key(api_key);
            Ok(ApiKeyValidationResult::valid_with_config(
                api_key.to_string(),
                config.clone(),
            ))
        } else {
            Ok(ApiKeyValidationResult::invalid())
        }
    }

    async fn get_rate_limit_config(&self, api_key: &str) -> Option<BarnacleConfig> {
        if self.config.key_configs.contains_key(api_key) {
            Some(self.config.get_config_for_key(api_key).clone())
        } else {
            None
        }
    }
}
