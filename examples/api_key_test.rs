use std::time::Duration;

use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
    routing::post,
    Router,
};
use barnacle_rs::{
    create_api_key_layer_with_config, ApiKeyMiddlewareConfig, BarnacleConfig, RedisApiKeyStore,
    RedisBarnacleStore, ResetOnSuccess,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct TestResponse {
    message: String,
    remaining: Option<u32>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt().with_env_filter("debug").init();

    // Create Redis stores
    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());

    let store = RedisBarnacleStore::from_url(&redis_url)
        .map_err(|e| format!("Failed to create Redis store: {}", e))?;

    // Create a separate API key store with its own pool
    let api_key_store = RedisApiKeyStore::from_url(&redis_url)
        .map_err(|e| format!("Failed to create API key store: {}", e))?;

    // Configure rate limiting - NO reset on success
    let config = ApiKeyMiddlewareConfig {
        header_name: "x-api-key".to_string(),
        barnacle_config: BarnacleConfig {
            max_requests: 3,
            window: Duration::from_secs(60),
            reset_on_success: ResetOnSuccess::Not, // This is the key change
        },
        require_api_key: true,
    };

    // Save a test API key to Redis with the same config
    api_key_store
        .save_key("test_key_123", Some(&config.barnacle_config), Some(30))
        .await
        .unwrap();

    tracing::info!("Saved API key with config: {:?}", config.barnacle_config);

    let middleware = create_api_key_layer_with_config(api_key_store, store, config);

    let app = Router::new().route("/test", post(test_endpoint).layer(middleware));

    tracing::info!("🚀 API Key Rate Limiter Test Server");
    tracing::info!("Server running on http://localhost:3001");
    tracing::info!(
        "Test with: curl -H 'x-api-key: test_key_123' -X POST http://localhost:3001/test"
    );

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3001").await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn test_endpoint(headers: HeaderMap) -> impl IntoResponse {
    let remaining = headers
        .get("X-RateLimit-Remaining")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u32>().ok());

    tracing::info!("Request processed, remaining: {:?}", remaining);

    (
        StatusCode::OK,
        Json(TestResponse {
            message: "Request successful".to_string(),
            remaining,
        }),
    )
}
