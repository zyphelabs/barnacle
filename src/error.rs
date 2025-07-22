use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

/// Main error type for the Barnacle library
#[derive(Error, Debug)]
pub enum BarnacleError {
    /// Rate limit exceeded error
    #[error("Rate limit exceeded: {remaining} requests remaining, retry after {retry_after}s")]
    RateLimitExceeded {
        remaining: u32,
        retry_after: u64,
        limit: u32,
    },

    /// API key validation errors
    #[error("API key validation failed: {reason}")]
    ApiKeyValidation { reason: String },

    /// Missing API key when required
    #[error("API key is required but not provided")]
    ApiKeyMissing,

    /// Invalid API key format or value
    #[error("Invalid API key: {key_hint}")]
    InvalidApiKey { key_hint: String },

    /// Store/backend related errors
    #[error("Backend store error: {message}")]
    StoreError {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// Redis-specific errors (when Redis feature is enabled)
    #[cfg(feature = "redis")]
    #[error("Redis error: {message}")]
    Redis {
        message: String,
        #[source]
        source: redis::RedisError,
    },

    /// Connection pool errors
    #[error("Connection pool error: {message}")]
    ConnectionPool {
        message: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Configuration errors
    #[error("Configuration error: {message}")]
    Configuration { message: String },

    /// JSON serialization/deserialization errors
    #[error("JSON processing error: {message}")]
    JsonError {
        message: String,
        #[source]
        source: serde_json::Error,
    },

    /// Request parsing errors
    #[error("Request parsing error: {message}")]
    RequestParsing { message: String },

    /// Internal server errors
    #[error("Internal server error: {message}")]
    Internal { message: String },

    /// Custom errors for extending functionality
    #[error("Custom error: {message}")]
    Custom {
        message: String,
        status_code: Option<StatusCode>,
    },

    #[error("Permission denied: {reason}")]
    PermissionDenied { reason: String },
}

impl BarnacleError {
    /// Create a rate limit exceeded error
    pub fn rate_limit_exceeded(remaining: u32, retry_after: u64, limit: u32) -> Self {
        Self::RateLimitExceeded {
            remaining,
            retry_after,
            limit,
        }
    }

    /// Create an API key validation error
    pub fn api_key_validation<S: Into<String>>(reason: S) -> Self {
        Self::ApiKeyValidation {
            reason: reason.into(),
        }
    }

    /// Create an invalid API key error with a hint (truncated key for security)
    pub fn invalid_api_key<S: Into<String>>(key: S) -> Self {
        let key_str = key.into();
        let key_hint = if key_str.len() > 8 {
            format!("{}...", &key_str[..8])
        } else {
            key_str
        };
        Self::InvalidApiKey { key_hint }
    }

    /// Create a store error
    pub fn store_error<S: Into<String>>(message: S) -> Self {
        Self::StoreError {
            message: message.into(),
            source: None,
        }
    }

    /// Create a store error with source
    pub fn store_error_with_source<S: Into<String>>(
        message: S,
        source: Box<dyn std::error::Error + Send + Sync>,
    ) -> Self {
        Self::StoreError {
            message: message.into(),
            source: Some(source),
        }
    }

    /// Create a Redis error (only available with redis feature)
    #[cfg(feature = "redis")]
    pub fn redis_error<S: Into<String>>(message: S, source: redis::RedisError) -> Self {
        Self::Redis {
            message: message.into(),
            source,
        }
    }

    /// Create a connection pool error
    pub fn connection_pool_error<S: Into<String>>(
        message: S,
        source: Box<dyn std::error::Error + Send + Sync>,
    ) -> Self {
        Self::ConnectionPool {
            message: message.into(),
            source,
        }
    }

    /// Create a configuration error
    pub fn configuration_error<S: Into<String>>(message: S) -> Self {
        Self::Configuration {
            message: message.into(),
        }
    }

    /// Create a JSON error
    pub fn json_error<S: Into<String>>(message: S, source: serde_json::Error) -> Self {
        Self::JsonError {
            message: message.into(),
            source,
        }
    }

    /// Create a request parsing error
    pub fn request_parsing_error<S: Into<String>>(message: S) -> Self {
        Self::RequestParsing {
            message: message.into(),
        }
    }

    /// Create an internal server error
    pub fn internal_error<S: Into<String>>(message: S) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }

    /// Create a custom error with optional status code
    pub fn custom<S: Into<String>>(message: S, status_code: Option<StatusCode>) -> Self {
        Self::Custom {
            message: message.into(),
            status_code,
        }
    }

    /// Get the appropriate HTTP status code for this error
    pub fn status_code(&self) -> StatusCode {
        match self {
            BarnacleError::RateLimitExceeded { .. } => StatusCode::TOO_MANY_REQUESTS,
            BarnacleError::ApiKeyValidation { .. } => StatusCode::UNAUTHORIZED,
            BarnacleError::ApiKeyMissing => StatusCode::UNAUTHORIZED,
            BarnacleError::InvalidApiKey { .. } => StatusCode::UNAUTHORIZED,
            BarnacleError::StoreError { .. } => StatusCode::SERVICE_UNAVAILABLE,
            #[cfg(feature = "redis")]
            BarnacleError::Redis { .. } => StatusCode::SERVICE_UNAVAILABLE,
            BarnacleError::ConnectionPool { .. } => StatusCode::SERVICE_UNAVAILABLE,
            BarnacleError::Configuration { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            BarnacleError::JsonError { .. } => StatusCode::BAD_REQUEST,
            BarnacleError::RequestParsing { .. } => StatusCode::BAD_REQUEST,
            BarnacleError::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            BarnacleError::PermissionDenied { .. } => StatusCode::FORBIDDEN,
            BarnacleError::Custom { status_code, .. } => {
                status_code.unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
    }

    /// Check if this error should be retried
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            BarnacleError::RateLimitExceeded { .. }
                | BarnacleError::StoreError { .. }
                | BarnacleError::ConnectionPool { .. }
        )
    }

    /// Get retry-after value in seconds if applicable
    pub fn retry_after(&self) -> Option<u64> {
        match self {
            BarnacleError::RateLimitExceeded { retry_after, .. } => Some(*retry_after),
            _ => None,
        }
    }

    /// Convert this error into a JSON representation
    pub fn to_json_value(&self) -> serde_json::Value {
        let mut json = json!({
            "error": {
                "code": self.error_code(),
                "message": self.to_string(),
                "type": self.error_type(),
            }
        });

        // Add specific fields for certain error types
        match self {
            BarnacleError::RateLimitExceeded {
                remaining,
                retry_after,
                limit,
            } => {
                json["error"]["details"] = json!({
                    "remaining": remaining,
                    "retry_after": retry_after,
                    "limit": limit
                });
            }
            BarnacleError::Custom { .. } => {
                // Allow custom errors to provide additional context
                json["error"]["details"] = json!({});
            }
            _ => {}
        }

        json
    }

    /// Get a unique error code for this error type
    pub fn error_code(&self) -> &'static str {
        match self {
            BarnacleError::RateLimitExceeded { .. } => "RATE_LIMIT_EXCEEDED",
            BarnacleError::ApiKeyValidation { .. } => "API_KEY_VALIDATION_FAILED",
            BarnacleError::ApiKeyMissing => "API_KEY_MISSING",
            BarnacleError::InvalidApiKey { .. } => "INVALID_API_KEY",
            BarnacleError::StoreError { .. } => "STORE_ERROR",
            #[cfg(feature = "redis")]
            BarnacleError::Redis { .. } => "REDIS_ERROR",
            BarnacleError::ConnectionPool { .. } => "CONNECTION_POOL_ERROR",
            BarnacleError::Configuration { .. } => "CONFIGURATION_ERROR",
            BarnacleError::JsonError { .. } => "JSON_ERROR",
            BarnacleError::RequestParsing { .. } => "REQUEST_PARSING_ERROR",
            BarnacleError::Internal { .. } => "INTERNAL_ERROR",
            BarnacleError::PermissionDenied { .. } => "PERMISSION_DENIED",
            BarnacleError::Custom { .. } => "CUSTOM_ERROR",
        }
    }

    /// Get the error type category
    pub fn error_type(&self) -> &'static str {
        match self {
            BarnacleError::RateLimitExceeded { .. } => "rate_limit",
            BarnacleError::ApiKeyValidation { .. }
            | BarnacleError::ApiKeyMissing
            | BarnacleError::InvalidApiKey { .. } => "authentication",
            BarnacleError::StoreError { .. } | BarnacleError::ConnectionPool { .. } => "backend",
            #[cfg(feature = "redis")]
            BarnacleError::Redis { .. } => "backend",
            BarnacleError::Configuration { .. } | BarnacleError::Internal { .. } => "server",
            BarnacleError::JsonError { .. } | BarnacleError::RequestParsing { .. } => "client",
            BarnacleError::PermissionDenied { .. } => "authentication",
            BarnacleError::Custom { .. } => "custom",
        }
    }
}

/// Helper function to safely convert values to HeaderValue
fn to_header_value<T: ToString>(value: T) -> axum::http::HeaderValue {
    value
        .to_string()
        .parse()
        .unwrap_or_else(|_| axum::http::HeaderValue::from_static("0"))
}

/// Implement IntoResponse for Axum integration
impl IntoResponse for BarnacleError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let mut response = Json(self.to_json_value()).into_response();

        // Set status code
        *response.status_mut() = status;

        // Add rate limit headers for rate limit errors
        if let BarnacleError::RateLimitExceeded {
            remaining,
            retry_after,
            limit,
        } = &self
        {
            let headers = response.headers_mut();
            headers.insert("X-RateLimit-Remaining", to_header_value(remaining));
            headers.insert("X-RateLimit-Limit", to_header_value(limit));
            // X-RateLimit-Reset follows Barnacle's convention: seconds until reset (same as Retry-After)
            headers.insert("X-RateLimit-Reset", to_header_value(retry_after));
        }

        response
            .headers_mut()
            .insert("X-Barnacle-Error", to_header_value("true"));

        response
    }
}

/// Result type alias for Barnacle operations
pub type BarnacleResult<T> = Result<T, BarnacleError>;

/// Convert from various error types into BarnacleError
impl From<serde_json::Error> for BarnacleError {
    fn from(err: serde_json::Error) -> Self {
        Self::json_error("JSON processing failed", err)
    }
}

impl From<anyhow::Error> for BarnacleError {
    fn from(err: anyhow::Error) -> Self {
        Self::internal_error(format!("Internal error: {}", err))
    }
}

#[cfg(feature = "redis")]
impl From<redis::RedisError> for BarnacleError {
    fn from(err: redis::RedisError) -> Self {
        Self::redis_error("Redis operation failed", err)
    }
}

#[cfg(feature = "redis")]
impl From<deadpool_redis::PoolError> for BarnacleError {
    fn from(err: deadpool_redis::PoolError) -> Self {
        Self::connection_pool_error("Redis pool error", Box::new(err))
    }
}

/// Extensions to provide additional error context
impl BarnacleError {
    /// Add additional context to an error
    pub fn with_context<S: Into<String>>(mut self, context: S) -> Self {
        let ctx = context.into();
        match &mut self {
            BarnacleError::StoreError { message, .. } => {
                *message = format!("{}: {}", ctx, message);
            }
            BarnacleError::Internal { message } => {
                *message = format!("{}: {}", ctx, message);
            }
            BarnacleError::Custom { message, .. } => {
                *message = format!("{}: {}", ctx, message);
            }
            _ => {
                // For other error types, convert to custom error with context
                return BarnacleError::custom(
                    format!("{}: {}", ctx, self),
                    Some(self.status_code()),
                );
            }
        }
        self
    }
}
