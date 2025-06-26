use async_trait::async_trait;
use deadpool_redis::redis::AsyncCommands;
use deadpool_redis::{Connection, Pool};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing;

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

/// Redis-based API key store for validation
pub struct RedisApiKeyStore {
    pool: Pool,
    default_config: BarnacleConfig,
    key_prefix: String,
}

impl RedisApiKeyStore {
    pub fn new(pool: Pool, default_config: BarnacleConfig) -> Self {
        Self {
            pool,
            default_config,
            key_prefix: "barnacle:api_keys".to_string(),
        }
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
}

#[async_trait]
impl ApiKeyStore for RedisApiKeyStore {
    async fn validate_key(&self, api_key: &str) -> ApiKeyValidationResult {
        let redis_key = self.get_redis_key(api_key);
        let config_key = self.get_config_key(api_key);

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
            tracing::debug!("API key not found: {}", api_key);
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

/// In-memory API key store for simple setups
pub struct InMemoryApiKeyStore {
    keys: Arc<RwLock<HashMap<String, BarnacleConfig>>>,
    default_config: BarnacleConfig,
}

impl InMemoryApiKeyStore {
    pub fn new(default_config: BarnacleConfig) -> Self {
        Self {
            keys: Arc::new(RwLock::new(HashMap::new())),
            default_config,
        }
    }

    pub async fn add_key(&self, api_key: String, config: BarnacleConfig) {
        let mut keys = self.keys.write().await;
        keys.insert(api_key, config);
    }

    pub async fn remove_key(&self, api_key: &str) {
        let mut keys = self.keys.write().await;
        keys.remove(api_key);
    }

    pub async fn from_static_config(static_config: StaticApiKeyConfig) -> Self {
        let store = Self::new(static_config.default_config);
        let mut keys = store.keys.write().await;
        *keys = static_config.key_configs;
        drop(keys);
        store
    }
}

#[async_trait]
impl ApiKeyStore for InMemoryApiKeyStore {
    async fn validate_key(&self, api_key: &str) -> ApiKeyValidationResult {
        let keys = self.keys.read().await;

        if let Some(config) = keys.get(api_key) {
            ApiKeyValidationResult::valid_with_config(api_key.to_string(), config.clone())
        } else {
            ApiKeyValidationResult::invalid()
        }
    }

    async fn get_rate_limit_config(&self, api_key: &str) -> Option<BarnacleConfig> {
        let keys = self.keys.read().await;
        keys.get(api_key).cloned()
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
