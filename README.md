# Barnacle - Rate Limiting Library

Barnacle is a custom rate limiting library for Rust, designed as an alternative to Tower Governor. It offers flexibility for different storage types (Redis, memory) and native integration with Axum.

## Features

- ✅ **Multiple key types**: IP, Email, API Key
- ✅ **Flexible configuration**: Time windows, request limits, backoff
- ✅ **Multiple backends**: Redis and in-memory storage
- ✅ **Axum integration**: Native middleware for Axum
- ✅ **Manual reset**: Ability to reset rate limits
- ✅ **Unit tests**: Complete test coverage

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
barnacle = { path = "./barnacle" }
```

## Basic Usage

### 1. Simple Example with Redis

```rust
use std::sync::Arc;
use std::time::Duration;

use axum::{Router, routing::get};
use barnacle::{
    barnacle_layer, redis_store::RedisBarnacleStore, types::BarnacleConfig,
};
use redis::Client;

#[tokio::main]
async fn main() {
    // Connect to Redis
    let redis_client = Arc::new(Client::open("redis://127.0.0.1/").unwrap());
    let store = Arc::new(RedisBarnacleStore::new(redis_client));

    // Configure rate limiting
    let config = BarnacleConfig {
        max_requests: 10,
        window: Duration::from_secs(60),
        backoff: None,
        reset_on_success: false,
    };

    // Create middleware
    let barnacle = barnacle_layer(store, config);

    // Apply to router
    let app = Router::new()
        .route("/", get(hello_world))
        .layer(barnacle);

    // Run server
    axum::serve(
        tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap(),
        app
    ).await.unwrap();
}

async fn hello_world() -> &'static str {
    "Hello, World!"
}
```

### 2. Example with In-Memory Storage (for testing)

```rust
use barnacle::{barnacle_layer, MemoryBarnacleStore, types::BarnacleConfig};

let store = Arc::new(MemoryBarnacleStore::new());
let config = BarnacleConfig {
    max_requests: 5,
    window: Duration::from_secs(60),
    backoff: None,
    reset_on_success: false,
};

let barnacle = barnacle_layer(store, config);
```

## Configuration

### BarnacleConfig

```rust
pub struct BarnacleConfig {
    pub max_requests: u32,           // Maximum number of requests
    pub window: Duration,            // Time window
    pub backoff: Option<Vec<Duration>>, // Optional exponential backoff
    pub reset_on_success: bool,      // Reset limit on success
}
```

### Key Types

```rust
pub enum BarnacleKey {
    Email(String),    // Rate limit by email
    ApiKey(String),   // Rate limit by API key
    Ip(String),       // Rate limit by IP
}
```

## Testing Features

### 1. Run Unit Tests

```bash
cargo test
```

### 2. Run Basic Example

```bash
# Start Redis first
redis-server

# In another terminal
cargo run --example basic
```

### 3. Run Advanced Example

```bash
cargo run --example advanced
```

### 4. Run Automated Test Script

```bash
# In one terminal, start the server
cargo run --example advanced

# In another terminal, run the tests
./test_rate_limiter.sh
```

## Test Scenarios

### Test 1: Basic Rate Limiting

- **Endpoint**: `GET /api/strict`
- **Limit**: 5 requests per minute
- **Behavior**: First 5 requests return 200, 6th returns 429

### Test 2: Different Limits

- **Endpoint**: `GET /api/moderate`
- **Limit**: 20 requests per minute
- **Behavior**: Allows more requests than the strict endpoint

### Test 3: Login Rate Limiting

- **Endpoint**: `POST /api/login`
- **Limit**: 3 attempts per 5 minutes
- **Behavior**: Blocks after 3 failed attempts

### Test 4: Manual Reset

- **Endpoint**: `POST /api/reset/:key_type/:value`
- **Behavior**: Resets rate limit for specific key

## Axum Integration

The Barnacle middleware is compatible with Axum's layer system:

```rust
use axum::Router;
use barnacle::barnacle_layer;

let app = Router::new()
    .route("/api/protected", get(protected_handler))
    .layer(barnacle_layer(store, config));
```

## Comparison with Tower Governor

| Feature | Barnacle | Tower Governor |
|---------|----------|----------------|
| Key flexibility | ✅ Multiple types | ❌ IP only |
| Customizable backend | ✅ Redis, Memory | ❌ Redis only |
| Manual reset | ✅ Yes | ❌ No |
| Axum integration | ✅ Native | ✅ Native |
| Configuration | ✅ Flexible | ✅ Flexible |

## Development

### Project Structure

```
barnacle/
├── src/
│   ├── lib.rs          # Main trait and in-memory store
│   ├── types.rs        # Data types
│   ├── middleware.rs   # Axum middleware
│   ├── redis_store.rs  # Redis implementation
│   └── backoff.rs      # Backoff strategies
├── examples/
│   ├── basic.rs        # Simple example
│   └── advanced.rs     # Complete example
└── tests/
    └── integration.rs  # Integration tests
```

### Adding New Backends

To add a new storage backend:

1. Implement the `BarnacleStore` trait
2. Add unit tests
3. Create usage example

```rust
#[async_trait]
impl BarnacleStore for MyCustomStore {
    async fn increment(&self, key: &BarnacleKey, config: &BarnacleConfig) -> BarnacleResult {
        // Custom implementation
    }

    async fn reset(&self, key: &BarnacleKey) -> anyhow::Result<()> {
        // Custom implementation
    }
}
```

## Contributing

1. Fork the project
2. Create a branch for your feature
3. Add tests for new functionality
4. Run `cargo test` to ensure everything works
5. Open a Pull Request

## License

MIT License - see the LICENSE file for details.
