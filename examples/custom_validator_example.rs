use async_trait::async_trait;
use axum::{response::Json, routing::get, Router};
use barnacle_rs::{
    create_api_key_layer_with_custom_validator, ApiKeyMiddlewareConfig, ApiKeyStore,
    ApiKeyValidationResult, BarnacleConfig, RedisApiKeyStore, RedisBarnacleStore,
};
use serde_json::json;
use std::time::Duration;
use tower::ServiceBuilder;

/// Example custom API key store that validates against a "database"
/// (in this case, just hardcoded keys for demonstration)
pub struct PostgresApiKeyStore {
    // In a real implementation, this would be a database connection pool
    valid_keys: std::collections::HashMap<String, String>,
}

impl PostgresApiKeyStore {
    pub fn new() -> Self {
        let mut valid_keys = std::collections::HashMap::new();
        valid_keys.insert("test-key-1".to_string(), "user_1".to_string());
        valid_keys.insert("test-key-2".to_string(), "user_2".to_string());
        valid_keys.insert("test-key-3".to_string(), "user_3".to_string());

        Self { valid_keys }
    }
}

impl Default for PostgresApiKeyStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ApiKeyStore for PostgresApiKeyStore {
    async fn validate_key(&self, api_key: &str) -> ApiKeyValidationResult {
        println!(
            "🔍 PostgresApiKeyStore: Validating key {} (simulating DB lookup)",
            api_key
        );

        // Simulate database latency
        tokio::time::sleep(Duration::from_millis(100)).await;

        if let Some(user_id) = self.valid_keys.get(api_key) {
            println!(
                "✅ PostgresApiKeyStore: Key {} is valid for user {}",
                api_key, user_id
            );
            ApiKeyValidationResult::valid_with_config(
                user_id.clone(),
                BarnacleConfig {
                    max_requests: 10,
                    window: Duration::from_secs(60),
                    reset_on_success: barnacle_rs::ResetOnSuccess::Not,
                },
            )
        } else {
            println!("❌ PostgresApiKeyStore: Key {} is invalid", api_key);
            ApiKeyValidationResult::invalid()
        }
    }
}

async fn protected_handler() -> Json<serde_json::Value> {
    Json(json!({
        "message": "Hello! This endpoint is protected by API key validation and rate limiting",
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

async fn health_handler() -> Json<serde_json::Value> {
    Json(json!({"status": "ok"}))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for better logging
    tracing_subscriber::fmt().init();

    println!("🚀 Starting Custom Validator Example");
    println!("📋 This example demonstrates:");
    println!("   1. Redis cache for fast API key validation");
    println!("   2. PostgreSQL fallback for unknown keys");
    println!("   3. Automatic caching of validated keys");
    println!();

    // Create Redis connection pool
    let redis_config = deadpool_redis::Config::from_url("redis://localhost:6379");
    let redis_pool = redis_config
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))
        .expect("Failed to create Redis pool");

    // Create the stores
    let redis_store = RedisApiKeyStore::new(redis_pool.clone());
    let postgres_store = PostgresApiKeyStore::new();
    let rate_limit_store = RedisBarnacleStore::new(redis_pool);

    // Configure the middleware
    let config = ApiKeyMiddlewareConfig {
        header_name: "x-api-key".to_string(),
        barnacle_config: BarnacleConfig {
            max_requests: 5, // Default rate limit if not specified by store
            window: Duration::from_secs(60),
            reset_on_success: barnacle_rs::ResetOnSuccess::Not,
        },
        require_api_key: true,
    };

    // Create the layer with custom validator
    let auth_layer = create_api_key_layer_with_custom_validator(
        redis_store,
        rate_limit_store,
        postgres_store,
        config,
    );

    // Build the router
    let app = Router::new()
        .route("/protected", get(protected_handler))
        .layer(ServiceBuilder::new().layer(auth_layer))
        .route("/health", get(health_handler)); // Health endpoint without auth

    println!("🌐 Server starting on http://localhost:3000");
    println!();
    println!("🧪 Test commands:");
    println!("   # Health check (no auth required)");
    println!("   curl http://localhost:3000/health");
    println!();
    println!("   # First request (will hit PostgreSQL, then cache in Redis)");
    println!("   curl -H 'x-api-key: test-key-1' http://localhost:3000/protected");
    println!();
    println!("   # Second request (will hit Redis cache, faster)");
    println!("   curl -H 'x-api-key: test-key-1' http://localhost:3000/protected");
    println!();
    println!("   # Invalid key");
    println!("   curl -H 'x-api-key: invalid-key' http://localhost:3000/protected");
    println!();
    println!("   # Rate limit test (make 6+ requests quickly)");
    println!("   for i in {{1..7}}; do curl -H 'x-api-key: test-key-1' http://localhost:3000/protected; echo; done");
    println!();

    // Start the server
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    axum::serve(listener, app).await?;

    Ok(())
}
