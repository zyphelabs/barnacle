use async_trait::async_trait;
#[cfg(feature = "redis")]
use deadpool_redis::redis::AsyncCommands;
#[cfg(feature = "redis")]
use deadpool_redis::{Connection, Pool};

use crate::types::{ApiKeyValidationResult, BarnacleConfig, StaticApiKeyConfig};

/// Trait for API key validation and configuration retrieval
#[async_trait]
pub trait ApiKeyStore: Send + Sync {
    /// Validate an API key and return its configuration
    async fn validate_key(&self, api_key: &str) -> ApiKeyValidationResult;

    /// Optional: Get rate limit configuration for a specific key
    /// This allows for dynamic per-key configuration
    async fn get_rate_limit_config(&self, api_key: &str) -> Option<BarnacleConfig> {
        // Default implementation returns None, letting validate_key handle config
        let _ = api_key;
        None
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
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let redis_key = self.get_redis_key(api_key);
        let config_key = self.get_config_key(api_key);

        tracing::debug!("Saving API key: {}", api_key);

        let mut conn = self.get_connection().await?;

        conn.set::<_, _, ()>(&redis_key, 1).await?;

        if let Some(cfg) = config {
            let config_json = serde_json::to_string(cfg)?;
            conn.set::<_, _, ()>(&config_key, config_json).await?;
        }

        Ok(())
    }
}

#[cfg(feature = "redis")]
#[async_trait]
impl ApiKeyStore for RedisApiKeyStore {
    async fn validate_key(&self, api_key: &str) -> ApiKeyValidationResult {
        let redis_key = self.get_redis_key(api_key);
        let config_key = self.get_config_key(api_key);

        tracing::debug!("Validating API key: {}", api_key);

        let mut conn = match self.get_connection().await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::error!("Redis connection error during API key validation: {}", e);
                return ApiKeyValidationResult::invalid();
            }
        };

        // Check if the API key exists
        let key_exists: bool = match conn.exists(&redis_key).await {
            Ok(exists) => exists,
            Err(e) => {
                tracing::error!("Redis EXISTS operation failed for API key: {}", e);
                return ApiKeyValidationResult::invalid();
            }
        };

        if !key_exists {
            tracing::warn!("API key not found: {}", api_key);
            return ApiKeyValidationResult::invalid();
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

        ApiKeyValidationResult::valid_with_config(api_key.to_string(), rate_limit_config)
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
impl ApiKeyStore for StaticApiKeyStore {
    async fn validate_key(&self, api_key: &str) -> ApiKeyValidationResult {
        if self.config.key_configs.contains_key(api_key) {
            let config = self.config.get_config_for_key(api_key);
            ApiKeyValidationResult::valid_with_config(api_key.to_string(), config.clone())
        } else {
            ApiKeyValidationResult::invalid()
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
