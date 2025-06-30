<div align="center">
  <img src="assets/barnacle-logo.png" alt="Barnacle Logo" width="200" style="border-radius: 15px;"/>
</div>

# Barnacle 🦀

A powerful and flexible rate limiting library for Rust with Redis backend support, designed primarily for Axum web applications.

## Features

- **🚦 Rate Limiting**: Configurable rate limiting with sliding window algorithm
- **🔐 API Key Validation**: Validate requests using `x-api-key` header with per-key rate limits
- **📊 Redis Integration**: Redis-based storage for distributed rate limiting
- **⚡ Axum Middleware**: Ready-to-use middleware for Axum web framework
- **🎯 Flexible Key Extraction**: Support for IP, email, custom and API key-based rate limiting
- **🔄 Reset on Success**: Optional rate limit reset on successful operations (e.g., successful login)
- **📈 Exponential Backoff**: Configurable backoff strategies
- **🎛️ Per-Key Configuration**: Different rate limits for different API keys

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
barnacle = "0.1.0"
tokio = { version = "1", features = ["full"] }
axum = "0.8"
redis = { version = "0.32.2", features = ["tokio-comp"] }
deadpool-redis = { version = "0.21.1", features = ["rt_tokio_1"] }
```

## Quick Start

### Basic Rate Limiting

```rust
use barnacle_rs::{create_barnacle_layer, RedisBarnacleStore, BarnacleConfig, ResetOnSuccess};
use axum::{Router, routing::get};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create Redis store (Arc is handled internally)
    let store = RedisBarnacleStore::from_url("redis://127.0.0.1:6379").await?;

    // Configure rate limiting (10 requests per minute)
    let config = BarnacleConfig {
        max_requests: 10,
        window: Duration::from_secs(60),
        reset_on_success: ResetOnSuccess::Not,
    };

    // Create rate limiting middleware
    let rate_limiter = create_barnacle_layer(store, config);

    // Apply to your Axum router
    let app = Router::new()
        .route("/api/data", get(handler))
        .layer(rate_limiter);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn handler() -> &'static str {
    "Hello, World!"
}
```

### API Key Validation with Rate Limiting

```rust
use barnacle_rs::{
    create_api_key_layer, RedisApiKeyStore, RedisBarnacleStore,
    BarnacleConfig
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create Redis pool
    let redis_pool = deadpool_redis::Config::from_url("redis://localhost")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;

    // Create stores (Arc is handled internally)
    let api_key_store = RedisApiKeyStore::new(
        redis_pool.clone(),
        BarnacleConfig::default()
    );
    let rate_limit_store = RedisBarnacleStore::new(redis_pool);

    // Create API key middleware
    let middleware = create_api_key_layer(api_key_store, rate_limit_store);

    // Use with Axum router
    let app = Router::new()
        .route("/api/protected", get(protected_handler))
        .layer(middleware);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn protected_handler() -> &'static str {
    "This endpoint requires a valid API key!"
}
```

### Custom Key Extraction (Email-based Rate Limiting)

```rust
use barnacle_rs::{KeyExtractable, BarnacleKey, create_barnacle_layer_for_payload};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

impl KeyExtractable for LoginRequest {
    fn extract_key(&self) -> BarnacleKey {
        BarnacleKey::Email(self.email.clone())
    }
}

// Rate limit login attempts by email (3 attempts per 20 seconds)
let login_config = BarnacleConfig {
    max_requests: 3,
    window: Duration::from_secs(20),
    reset_on_success: ResetOnSuccess::Yes(Some(vec![200])), // Reset on successful login
};

let login_limiter = create_barnacle_layer_for_payload::<LoginRequest>(
    store.clone(), 
    login_config
);

let app = Router::new()
    .route("/api/login", post(login_handler))
    .layer(login_limiter);
```

## Configuration

### BarnacleConfig

```rust
use barnacle_rs::{BarnacleConfig, ResetOnSuccess};
use std::time::Duration;

let config = BarnacleConfig {
    max_requests: 100,                              // Maximum requests allowed
    window: Duration::from_secs(3600),              // Time window (1 hour)
    reset_on_success: ResetOnSuccess::Yes(          // Reset counter on success
        Some(vec![200, 201])                        // Only for these status codes
    ),
};
```

### Rate Limiting Strategies

#### IP-based Rate Limiting

```rust
// Automatically uses client IP for rate limiting
let ip_limiter = create_barnacle_layer(store, config);
```

#### API Key-based Rate Limiting

```rust
// Uses x-api-key header for rate limiting  
let api_limiter = create_api_key_layer(api_key_store, rate_limit_store);
```

#### Custom Key-based Rate Limiting

```rust
// Rate limit by any extractable key (email, user ID, etc.)
let custom_limiter = create_barnacle_layer_for_payload::<CustomPayload>(store, config);
```

## Rate Limit Headers

Barnacle automatically adds standard rate limit headers to responses:

- `X-RateLimit-Limit`: Maximum number of requests allowed
- `X-RateLimit-Remaining`: Number of requests remaining in current window
- `X-RateLimit-Reset`: Unix timestamp when the rate limit resets

## API Key Management

### Setting up API Keys in Redis

```rust
// Store API key
let key = "barnacle:api_keys:your-api-key";
let _: () = redis::cmd("SET")
    .arg(key)
    .arg(1)
    .query_async(&mut conn)
    .await?;

// Store per-key rate limit configuration
let config_key = "barnacle:api_keys:config:your-api-key";
let config = BarnacleConfig {
    max_requests: 1000,
    window: Duration::from_secs(3600),
    reset_on_success: ResetOnSuccess::Not,
};
let config_json = serde_json::to_string(&config)?;
let _: () = redis::cmd("SET")
    .arg(config_key)
    .arg(config_json)
    .query_async(&mut conn)
    .await?;
```

### Static API Keys (for testing)

```rust
use barnacle_rs::{StaticApiKeyStore, StaticApiKeyConfig};
use std::collections::HashMap;

let mut api_keys = HashMap::new();
api_keys.insert("test-key-123".to_string(), StaticApiKeyConfig {
    config: BarnacleConfig::default(),
});

let static_store = StaticApiKeyStore::new(api_keys);
```

## Examples

The `examples/` directory contains complete working examples:

- **`basic.rs`**: Demonstrates different rate limiting configurations and email-based rate limiting
- **`api_key_redis_test.rs`**: Shows API key validation with Redis backend

### Running Examples

```bash
# Start Redis
docker run -d -p 6379:6379 redis:alpine

# Run basic example
cargo run --example basic

# Run API key example
cargo run --example api_key_redis_test

# Test with curl
curl -H "x-api-key: valid-key-123" http://localhost:3000/test
```

## Error Handling

Barnacle provides comprehensive error handling:

```rust
use barnacle_rs::BarnacleResult;

match result {
    Ok(rate_limit_info) => {
        println!("Requests remaining: {}", rate_limit_info.remaining_requests);
    }
    Err(e) => {
        eprintln!("Rate limiting error: {}", e);
    }
}
```

## Redis Schema

Barnacle uses the following Redis key patterns:

- `barnacle:api_keys:{key}`: API key validity (1 = valid)
- `barnacle:api_keys:config:{key}`: Per-key rate limit configuration (JSON)
- `barnacle:rate_limit:{key}`: Rate limit counters and timestamps

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Performance

Barnacle is designed for high-performance scenarios:

- **Redis Pipeline**: Efficient batch operations
- **Connection Pooling**: Reuses Redis connections
- **Async/Await**: Non-blocking I/O operations
- **Minimal Overhead**: Lightweight middleware design
