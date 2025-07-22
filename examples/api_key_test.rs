use std::time::Duration;

use axum::{
    http::{request::Parts, HeaderMap, StatusCode},
    response::{IntoResponse, Json},
    routing::post,
    Router,
};
use serde::{Deserialize, Serialize};
use barnacle_rs::{ApiKeyConfig, BarnacleConfig, BarnacleLayer, RedisBarnacleStore};
use std::sync::Arc;
use barnacle_rs::BarnacleError;

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

    let config = BarnacleConfig {
        max_requests: 3,
        window: Duration::from_secs(60),
        reset_on_success: barnacle_rs::ResetOnSuccess::Not,
    };

    let api_key_validator = |api_key: String, _api_key_config: ApiKeyConfig, _parts: Arc<Parts>, _state: ()| async move {
        if api_key.is_empty() {
            Err(BarnacleError::ApiKeyMissing)
        } else if api_key != "test-key" {
            Err(BarnacleError::invalid_api_key(api_key))
        } else {
            Ok(())
        }
    };

    let middleware: BarnacleLayer<(), _, _, BarnacleError, _> = BarnacleLayer::builder()
        .with_store(store)
        .with_config(config)
        .with_api_key_validator(api_key_validator)
        .with_state(())
        .build()
        .unwrap();

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
