use std::sync::Arc;
use std::time::Duration;

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::Json,
    routing::{get, post},
};
use barnacle::{
    BarnacleStore, barnacle_layer,
    redis_store::RedisBarnacleStore,
    types::{BarnacleConfig, BarnacleKey},
};
use serde::{Deserialize, Serialize};

#[tokio::main]
async fn main() {
    // Create in-memory store for testing
    // Create Redis client
    let redis_client = Arc::new(
        redis::Client::open("redis://127.0.0.1:6379").expect("Failed to connect to Redis"),
    );

    // Create Redis store
    let store = Arc::new(RedisBarnacleStore::new(redis_client));
    let state = AppState {
        store: store.clone(),
    };

    // Configure rate limiting
    let config = BarnacleConfig {
        max_requests: 5,
        window: Duration::from_secs(60),
        backoff: None,
        reset_on_success: false,
    };

    // Create rate limiting layer
    let rate_limiter = barnacle_layer(store.clone(), config);

    // Build the application
    let app = Router::new()
        .route("/api/test", get(test_endpoint).layer(rate_limiter))
        .route("/api/reset/{:key_type}/{:value}", post(reset_rate_limit))
        .with_state(state);

    println!("🚀 Barnacle Rate Limiter Basic Demo");
    println!("===================================");
    println!("Available endpoints:");
    println!("  GET  /api/test      - Test endpoint with rate limiting (5 req/min)");
    println!("  POST /api/reset/key_type/value - Reset rate limit for specific key");
    println!();
    println!("Server running on http://localhost:3000");
    println!("Press Ctrl+C to stop");

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn test_endpoint(headers: HeaderMap) -> Json<ApiResponse> {
    let rate_limit_info = extract_rate_limit_info(&headers);

    Json(ApiResponse {
        message: "This endpoint has rate limiting (5 requests per minute)".to_string(),
        remaining_requests: rate_limit_info.as_ref().map(|info| info.remaining),
        rate_limit_info,
    })
}

async fn reset_rate_limit(
    State(state): State<AppState>,
    axum::extract::Path((key_type, value)): axum::extract::Path<(String, String)>,
) -> Result<Json<ApiResponse>, StatusCode> {
    let key = match key_type.as_str() {
        "email" => BarnacleKey::Email(value.clone()),
        "ip" => BarnacleKey::Ip(value.clone()),
        "apikey" => BarnacleKey::ApiKey(value.clone()),
        _ => return Err(StatusCode::BAD_REQUEST),
    };

    match state.store.reset(&key).await {
        Ok(_) => Ok(Json(ApiResponse {
            message: format!("Rate limit reset for {}: {}", key_type, value),
            remaining_requests: None,
            rate_limit_info: None,
        })),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

#[derive(Serialize, Deserialize)]
struct ApiResponse {
    message: String,
    remaining_requests: Option<u32>,
    rate_limit_info: Option<RateLimitInfo>,
}

#[derive(Serialize, Deserialize)]
struct RateLimitInfo {
    remaining: u32,
    limit: u32,
    reset_after: Option<u64>,
}

// Shared state for the application
#[derive(Clone)]
struct AppState {
    store: Arc<RedisBarnacleStore>,
}

// Helper function to extract rate limit info from headers
fn extract_rate_limit_info(headers: &HeaderMap) -> Option<RateLimitInfo> {
    let remaining = headers
        .get("X-RateLimit-Remaining")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u32>().ok())?;

    let limit = headers
        .get("X-RateLimit-Limit")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u32>().ok())?;

    let reset_after = headers
        .get("X-RateLimit-Reset")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());

    Some(RateLimitInfo {
        remaining,
        limit,
        reset_after,
    })
}
