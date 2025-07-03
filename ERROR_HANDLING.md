# Error Handling in Barnacle-RS

This document describes the comprehensive error handling system implemented in Barnacle-RS using the `thiserror` library with full Axum integration.

## Overview

Barnacle-RS provides a structured error handling system through the `BarnacleError` enum, which:

- Uses `thiserror` for automatic error trait implementations
- Implements `IntoResponse` for seamless Axum integration  
- Provides proper HTTP status codes and headers
- Allows consuming applications to convert errors to their own types
- Includes comprehensive error context and metadata

## Error Types

### Core Error Variants

```rust
pub enum BarnacleError {
    /// Rate limit exceeded with detailed information
    RateLimitExceeded {
        remaining: u32,
        retry_after: u64,
        limit: u32,
    },

    /// API key validation failed
    ApiKeyValidation { reason: String },

    /// Missing API key when required
    ApiKeyMissing,

    /// Invalid API key format or value
    InvalidApiKey { key_hint: String },

    /// Backend store errors (Redis, database, etc.)
    StoreError {
        message: String,
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// Redis-specific errors (when redis feature enabled)
    #[cfg(feature = "redis")]
    Redis {
        message: String,
        source: redis::RedisError,
    },

    /// Connection pool errors
    ConnectionPool {
        message: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Configuration errors
    Configuration { message: String },

    /// JSON processing errors
    JsonError {
        message: String,
        source: serde_json::Error,
    },

    /// Request parsing errors
    RequestParsing { message: String },

    /// Internal server errors
    Internal { message: String },

    /// Custom errors for extending functionality
    Custom {
        message: String,
        status_code: Option<StatusCode>,
    },
}
```

## HTTP Status Code Mapping

Each error variant maps to an appropriate HTTP status code:

| Error Type | HTTP Status | Description |
|------------|-------------|-------------|
| `RateLimitExceeded` | 429 Too Many Requests | Rate limit exceeded |
| `ApiKeyValidation` | 401 Unauthorized | API key validation failed |
| `ApiKeyMissing` | 401 Unauthorized | Missing required API key |
| `InvalidApiKey` | 401 Unauthorized | Invalid API key format/value |
| `StoreError` | 503 Service Unavailable | Backend store unavailable |
| `Redis` | 503 Service Unavailable | Redis operation failed |
| `ConnectionPool` | 503 Service Unavailable | Connection pool exhausted |
| `Configuration` | 500 Internal Server Error | Configuration error |
| `JsonError` | 400 Bad Request | JSON parsing failed |
| `RequestParsing` | 400 Bad Request | Request format invalid |
| `Internal` | 500 Internal Server Error | Internal server error |
| `Custom` | Configurable | Application-specific error |

## JSON Response Format

All errors are automatically converted to structured JSON responses:

```json
{
  "error": {
    "code": "RATE_LIMIT_EXCEEDED",
    "message": "Rate limit exceeded: 0 requests remaining, retry after 60s",
    "type": "rate_limit",
    "details": {
      "remaining": 0,
      "retry_after": 60,
      "limit": 100
    }
  }
}
```

### Rate Limit Headers

Rate limit errors automatically include proper HTTP headers:

```
X-RateLimit-Remaining: 0
X-RateLimit-Limit: 100
X-RateLimit-Reset: 60
Retry-After: 60
```

## Creating Errors

### Constructor Methods

```rust
// Rate limit error
let error = BarnacleError::rate_limit_exceeded(0, 60, 100);

// API key errors
let error = BarnacleError::ApiKeyMissing;
let error = BarnacleError::invalid_api_key("abc123...");
let error = BarnacleError::api_key_validation("Key expired");

// Store errors
let error = BarnacleError::store_error("Redis connection failed");
let error = BarnacleError::store_error_with_source("Database error", Box::new(db_error));

// Configuration errors
let error = BarnacleError::configuration_error("Invalid rate limit setting");

// Custom errors
let error = BarnacleError::custom("Application-specific error", Some(StatusCode::CONFLICT));

// Internal errors
let error = BarnacleError::internal_error("Unexpected failure");
```

### From Conversions

Automatic conversions from common error types:

```rust
// From serde_json::Error
let json_error: serde_json::Error = /* ... */;
let barnacle_error: BarnacleError = json_error.into();

// From anyhow::Error  
let anyhow_error: anyhow::Error = /* ... */;
let barnacle_error: BarnacleError = anyhow_error.into();

// From redis::RedisError (with redis feature)
let redis_error: redis::RedisError = /* ... */;
let barnacle_error: BarnacleError = redis_error.into();

// From deadpool_redis::PoolError (with redis feature)
let pool_error: deadpool_redis::PoolError = /* ... */;
let barnacle_error: BarnacleError = pool_error.into();
```

### Adding Context

```rust
let error = BarnacleError::store_error("Connection failed")
    .with_context("During user authentication");
```

## Application Integration

### Method 1: Using From Trait

```rust
use barnacle_rs::{BarnacleError, FromBarnacleError};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Rate limiting error: {0}")]
    RateLimit(#[from] BarnacleError),
    
    #[error("Database error: {0}")]
    Database(String),
    
    // ... other variants
}

// Automatic conversion
let app_error: AppError = barnacle_error.into();
```

### Method 2: Using FromBarnacleError Trait

```rust
use barnacle_rs::{BarnacleError, FromBarnacleError};

impl FromBarnacleError<AppError> for AppError {
    fn from_barnacle_error(error: BarnacleError) -> AppError {
        AppError::RateLimit(error)
    }
}

// Manual conversion
let app_error = AppError::from_barnacle_error(barnacle_error);
```

### Method 3: Using the Convenience Macro

```rust
use barnacle_rs::impl_from_barnacle_error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Barnacle error: {0}")]
    Barnacle(BarnacleError),
    // ... other variants
}

impl_from_barnacle_error!(AppError, Barnacle);
```

### Method 4: Manual Pattern Matching

```rust
fn handle_barnacle_error(error: BarnacleError) -> AppError {
    match error {
        BarnacleError::RateLimitExceeded { remaining, retry_after, limit } => {
            AppError::RateLimit {
                remaining,
                retry_after,
                limit,
                context: "API rate limit exceeded".to_string(),
            }
        },
        BarnacleError::ApiKeyMissing => {
            AppError::Authentication("API key required".to_string())
        },
        BarnacleError::InvalidApiKey { key_hint } => {
            AppError::Authentication(format!("Invalid API key: {}", key_hint))
        },
        other => AppError::Internal(format!("Service error: {}", other)),
    }
}
```

## Axum Handler Examples

### Basic Error Handling

```rust
use axum::{Json, response::Result};
use barnacle_rs::BarnacleError;

async fn handler() -> Result<Json<Value>, BarnacleError> {
    // Your logic here
    if some_condition {
        return Err(BarnacleError::rate_limit_exceeded(0, 60, 100));
    }
    
    Ok(Json(json!({"status": "success"})))
}
```

### With Application Error Type

```rust
async fn handler() -> Result<Json<Value>, AppError> {
    let result = some_barnacle_operation().await
        .map_err(AppError::from_barnacle_error)?;
    
    Ok(Json(json!({"data": result})))
}
```

### Error Conversion in Middleware

```rust
async fn error_middleware<B>(
    req: Request<B>,
    next: Next<B>,
) -> Result<Response, AppError> {
    match next.run(req).await {
        Ok(response) => Ok(response),
        Err(err) => {
            // Convert any BarnacleError to AppError
            if let Some(barnacle_err) = err.downcast_ref::<BarnacleError>() {
                Err(AppError::from_barnacle_error(barnacle_err.clone()))
            } else {
                Err(AppError::Internal("Unknown error".to_string()))
            }
        }
    }
}
```

## Error Inspection Methods

### Status and Metadata

```rust
let error = BarnacleError::rate_limit_exceeded(5, 30, 100);

// HTTP status code
assert_eq!(error.status_code(), StatusCode::TOO_MANY_REQUESTS);

// Error code for client identification
assert_eq!(error.error_code(), "RATE_LIMIT_EXCEEDED");

// Error category
assert_eq!(error.error_type(), "rate_limit");

// Retry information
assert!(error.is_retryable());
assert_eq!(error.retry_after(), Some(30));

// JSON representation
let json = error.to_json_value();
```

## Best Practices

### 1. Use Appropriate Error Types

```rust
// Good: Specific error for the situation
BarnacleError::invalid_api_key(api_key)

// Avoid: Generic internal error
BarnacleError::internal_error("Key validation failed")
```

### 2. Provide Context

```rust
// Good: Context helps debugging
BarnacleError::store_error("Redis connection failed")
    .with_context("During user session validation")

// Basic: Less context
BarnacleError::store_error("Redis error")
```

### 3. Handle Redis Feature Compilation

```rust
#[cfg(feature = "redis")]
fn handle_redis_error(err: redis::RedisError) -> BarnacleError {
    BarnacleError::redis_error("Redis operation failed", err)
}

#[cfg(not(feature = "redis"))]
fn handle_redis_error(err: SomeOtherError) -> BarnacleError {
    BarnacleError::store_error("Backend operation failed")
}
```

### 4. Security Considerations

```rust
// Good: Key hint for debugging, but not full key
BarnacleError::invalid_api_key("sk_test_abc123...")

// Avoid: Exposing full API key
BarnacleError::invalid_api_key("sk_test_abc123def456ghi789")
```

## Testing

### Unit Tests

```rust
#[test]
fn test_error_properties() {
    let error = BarnacleError::rate_limit_exceeded(5, 30, 100);
    
    assert_eq!(error.status_code(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(error.error_code(), "RATE_LIMIT_EXCEEDED");
    assert!(error.is_retryable());
    assert_eq!(error.retry_after(), Some(30));
}

#[test]
fn test_error_context() {
    let error = BarnacleError::store_error("Connection failed")
        .with_context("Database operation");
    
    assert!(error.to_string().contains("Database operation"));
    assert!(error.to_string().contains("Connection failed"));
}
```

### Integration Tests

```rust
#[tokio::test]
async fn test_error_response() {
    let app = Router::new().route("/test", get(|| async {
        Err::<Json<Value>, _>(BarnacleError::rate_limit_exceeded(0, 60, 100))
    }));

    let response = app
        .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    
    let headers = response.headers();
    assert_eq!(headers.get("Retry-After").unwrap(), "60");
    assert_eq!(headers.get("X-RateLimit-Remaining").unwrap(), "0");
}
```

## Migration from anyhow::Result

If you're migrating from the previous error handling:

### Before

```rust
async fn reset(&self, context: &BarnacleContext) -> anyhow::Result<()> {
    // implementation
}
```

### After

```rust
async fn reset(&self, context: &BarnacleContext) -> Result<(), BarnacleError> {
    // implementation with proper error conversion
}
```

The new system provides much better error information, proper HTTP responses, and enables consuming applications to handle errors appropriately.