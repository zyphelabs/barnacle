use std::collections::HashMap;
use std::time::Duration;

use axum::http::header::ORIGIN;

/// Special constant to indicate a placeholder key that should be replaced
pub const NO_KEY: &str = "__BARNACLE_NO_KEY_PLACEHOLDER__";

/// Result of API key extraction from a request
#[derive(Clone, Debug)]
pub struct ApiKeyExtractionResult<T = Vec<String>> {
    pub api_key: String,
    pub query_params: HashMap<String, String>,
    pub matched_path: String,
    pub original_path: String,
    pub extracted_data: Option<T>,
    pub origin: String,
    pub barnacle_context: BarnacleContext,
}

impl<T> ApiKeyExtractionResult<T> {
    pub fn new(
        api_key: String,
        query_params: HashMap<String, String>,
        matched_path: String,
        original_path: String,
        extracted_data: Option<T>,
        origin: String,
        barnacle_context: BarnacleContext,
    ) -> Self {
        Self {
            api_key,
            query_params,
            matched_path,
            original_path,
            extracted_data,
            origin,
            barnacle_context,
        }
    }

    /// New simple API: closure receives (original_path, matched_path) and returns Option<U>
    pub fn extract_request_data<B, U, F>(
        request: &axum::extract::Request<B>,
        header_name: &str,
        extractor: F,
    ) -> Result<ApiKeyExtractionResult<U>, crate::error::BarnacleError>
    where
        F: Fn(&str, &str) -> Option<U>,
    {
        // Extract API key from header using configurable header name
        let api_key = request
            .headers()
            .get(header_name)
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string())
            .ok_or(crate::error::BarnacleError::ApiKeyMissing)?;

        // Extract all query parameters
        let query_params = request
            .uri()
            .query()
            .map(|query_string| {
                let mut params = HashMap::new();
                for param in query_string.split('&') {
                    if let Some((key, value)) = param.split_once('=') {
                        params.insert(key.to_string(), value.to_string());
                    } else {
                        // Parameter without value (e.g., ?debug)
                        params.insert(param.to_string(), String::new());
                    }
                }
                params
            })
            .unwrap_or_default();

        // Extract matched path from request extensions
        let matched_path = request
            .extensions()
            .get::<axum::extract::MatchedPath>()
            .map(|p| p.as_str())
            .unwrap_or(request.uri().path())
            .to_string();

        // Extract original path from request extensions
        let original_path = request
            .extensions()
            .get::<axum::extract::OriginalUri>()
            .map(|original_uri| original_uri.path())
            .unwrap_or(request.uri().path())
            .to_string();

        // Use the extractor closure
        let extracted_data = extractor(&original_path, &matched_path);

        // Extract origin header
        let origin = request
            .headers()
            .get(ORIGIN)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("")
            .to_string();

        // Create context
        let barnacle_context = BarnacleContext {
            key: BarnacleKey::ApiKey(api_key.clone()),
            path: request.uri().path().to_string(),
            method: request.method().to_string(),
        };

        Ok(ApiKeyExtractionResult::new(
            api_key,
            query_params,
            matched_path,
            original_path,
            extracted_data,
            origin,
            barnacle_context,
        ))
    }
}

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
    pub fn new(max_requests: u32, window: Duration) -> Self {
        Self {
            max_requests,
            window,
            reset_on_success: ResetOnSuccess::Not,
        }
    }

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
pub struct ApiKeyValidationResult<T> {
    pub valid: bool,
    pub key: Option<T>,
    pub rate_limit_config: Option<BarnacleConfig>,
}

impl<T> ApiKeyValidationResult<T> {
    pub fn valid_with_config(key: T, config: BarnacleConfig) -> Self {
        Self {
            valid: true,
            key: Some(key),
            rate_limit_config: Some(config),
        }
    }

    pub fn valid_with_default_config(key: T) -> Self {
        Self {
            valid: true,
            key: Some(key),
            rate_limit_config: Some(BarnacleConfig::default()),
        }
    }

    pub fn invalid() -> Self {
        Self {
            valid: false,
            key: None,
            rate_limit_config: None,
        }
    }
}

impl ApiKeyValidationResult<String> {
    pub fn valid_string_key(key: String, config: BarnacleConfig) -> Self {
        Self::valid_with_config(key, config)
    }
}

/// Configuration for API key middleware
#[derive(Clone, Debug)]
pub struct ApiKeyMiddlewareConfig {
    pub header_name: String,
    pub barnacle_config: BarnacleConfig,
    pub require_api_key: bool,
    /// TTL for caching API keys validated by custom validator (in seconds)
    pub cache_ttl_seconds: u64,
}

impl ApiKeyMiddlewareConfig {
    pub fn new(barnacle_config: BarnacleConfig) -> Self {
        Self {
            header_name: "x-api-key".to_string(),
            barnacle_config,
            require_api_key: true,
            cache_ttl_seconds: 60 * 60, // 1 hour default
        }
    }

    pub fn custom(
        header_name: String,
        barnacle_config: BarnacleConfig,
        cache_ttl_seconds: u64,
    ) -> Self {
        Self {
            header_name,
            barnacle_config,
            require_api_key: true,
            cache_ttl_seconds, // 1 hour default
        }
    }
}

impl Default for ApiKeyMiddlewareConfig {
    fn default() -> Self {
        Self {
            header_name: "x-api-key".to_string(),
            barnacle_config: BarnacleConfig::default(),
            require_api_key: true,
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
