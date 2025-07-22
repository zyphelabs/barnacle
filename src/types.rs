use std::collections::HashMap;
use std::time::Duration;

/// Special constant to indicate a placeholder key that should be replaced
pub const NO_KEY: &str = "__BARNACLE_NO_KEY_PLACEHOLDER__";

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ResetOnSuccess {
    Not,
    Yes(Option<Vec<u16>>),
    Multiple(Option<Vec<u16>>, Vec<BarnacleContext>),
}

/// Rate limiter configuration
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct BarnacleConfig {
    pub max_requests: u32,
    pub window: Duration,
    pub reset_on_success: ResetOnSuccess,
}

impl Default for BarnacleConfig {
    fn default() -> Self {
        Self {
            max_requests: 20,
            window: Duration::from_secs(60), // 1 minute
            reset_on_success: ResetOnSuccess::Not,
        }
    }
}

impl BarnacleConfig {
    /// Check if a status code should be considered successful for rate limit reset
    pub fn is_success_status(&self, status_code: u16) -> bool {
        match &self.reset_on_success {
            ResetOnSuccess::Not => false,
            ResetOnSuccess::Yes(success_codes) | ResetOnSuccess::Multiple(success_codes, _) => {
                if let Some(codes) = success_codes {
                    codes.contains(&status_code)
                } else {
                    // Default to 2xx status codes
                    (200..300).contains(&status_code)
                }
            }
        }
    }
}

/// Identification key for rate limiting (e.g., email, api-key, IP)
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum BarnacleKey {
    Email(String),
    ApiKey(String),
    Ip(String),
    Custom(String),
}

/// Rate limiting context that includes route information
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct BarnacleContext {
    pub key: BarnacleKey,
    pub path: String,
    pub method: String,
}

impl BarnacleContext {
    /// This will be used to reset the rate limit for a specific path and method
    ///
    /// The key will be replaced with the current request's key
    pub fn with_path_and_method(path: impl Into<String>, method: impl Into<String>) -> Self {
        Self {
            key: BarnacleKey::Custom(NO_KEY.to_string()),
            path: path.into(),
            method: method.into(),
        }
    }
}

/// Result of an increment attempt
#[derive(Clone, Debug)]
pub struct BarnacleResult {
    pub allowed: bool,
    pub remaining: u32,
    pub retry_after: Option<Duration>,
}

/// API key validation result
#[derive(Clone, Debug)]
pub struct ApiKeyValidationResult {
    pub valid: bool,
    pub key_id: Option<String>,
    pub rate_limit_config: Option<BarnacleConfig>,
}

impl ApiKeyValidationResult {
    pub fn valid_with_config(key_id: String, config: BarnacleConfig) -> Self {
        Self {
            valid: true,
            key_id: Some(key_id),
            rate_limit_config: Some(config),
        }
    }

    pub fn valid_with_default_config(key_id: String) -> Self {
        Self {
            valid: true,
            key_id: Some(key_id),
            rate_limit_config: Some(BarnacleConfig::default()),
        }
    }

    pub fn invalid() -> Self {
        Self {
            valid: false,
            key_id: None,
            rate_limit_config: None,
        }
    }
}

/// Configuration for API key middleware
#[derive(Clone, Debug)]
pub struct ApiKeyConfig {
    pub header_name: String,
    /// TTL for caching API keys validated by custom validator (in seconds)
    pub cache_ttl_seconds: u64,
}

impl ApiKeyConfig {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn custom(
        header_name: String,
        cache_ttl_seconds: u64,
    ) -> Self {
        Self {
            header_name,
            cache_ttl_seconds, // 1 hour default
        }
    }
}

impl Default for ApiKeyConfig {
    fn default() -> Self {
        Self {
            header_name: "x-api-key".to_string(),
            cache_ttl_seconds: 60 * 60, // 1 hour default
        }
    }
}

/// Per-key rate limiting configuration for static configurations
#[derive(Clone, Debug)]
pub struct StaticApiKeyConfig {
    pub key_configs: HashMap<String, BarnacleConfig>,
    pub default_config: BarnacleConfig,
}

impl StaticApiKeyConfig {
    pub fn new(default_config: BarnacleConfig) -> Self {
        Self {
            key_configs: HashMap::new(),
            default_config,
        }
    }

    pub fn with_key_config(mut self, api_key: String, config: BarnacleConfig) -> Self {
        self.key_configs.insert(api_key, config);
        self
    }

    pub fn get_config_for_key(&self, api_key: &str) -> &BarnacleConfig {
        self.key_configs
            .get(api_key)
            .unwrap_or(&self.default_config)
    }
}
