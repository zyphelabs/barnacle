use std::sync::Arc;
use std::time::Duration;

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
    routing::{get, post},
};
use barnacle::redis_store::RedisBarnacleStore;
use barnacle::{
    BarnacleStore, barnacle_layer,
    types::{BarnacleConfig, BarnacleKey},
};
use serde::{Deserialize, Serialize};

#[tokio::main]
async fn main() {
    // Create Redis client
    let redis_client = Arc::new(
        redis::Client::open("redis://127.0.0.1:6379").expect("Failed to connect to Redis"),
    );

    // Create Redis store
    let store = Arc::new(RedisBarnacleStore::new(redis_client));
    let state = AppState {
        store: store.clone(),
    };

    // Configure different rate limiting rules
    let strict_config = BarnacleConfig {
        max_requests: 5,
        window: Duration::from_secs(60),
        backoff: None,
        reset_on_success: false,
    };

    let moderate_config = BarnacleConfig {
        max_requests: 20,
        window: Duration::from_secs(60),
        backoff: None,
        reset_on_success: false,
    };

    let login_config = BarnacleConfig {
        max_requests: 3,
        window: Duration::from_secs(300), // 5 minutes
        backoff: None,
        reset_on_success: true, // Reset on successful login
    };

    // Create rate limiting layers
    let strict_limiter = barnacle_layer(store.clone(), strict_config);
    let moderate_limiter = barnacle_layer(store.clone(), moderate_config);

    // Build the application with different rate limiting for different routes
    let app = Router::new()
        .route("/api/strict", get(strict_endpoint).layer(strict_limiter))
        .route(
            "/api/moderate",
            get(moderate_endpoint).layer(moderate_limiter),
        )
        .route("/api/login", post(login_endpoint))
        .route("/api/reset/{:key_type}/{:value}", post(reset_rate_limit))
        .route("/api/status", get(status_endpoint))
        .with_state(state);

    println!("🚀 Barnacle Rate Limiter Demo Server");
    println!("=====================================");
    println!("Available endpoints:");
    println!("  GET  /api/strict     - Strict rate limit (5 req/min)");
    println!("  GET  /api/moderate   - Moderate rate limit (20 req/min)");
    println!("  POST /api/login      - Login with rate limiting (3 req/5min)");
    println!("  POST /api/reset/key_type/value - Reset rate limit for specific key");
    println!("  GET  /api/status     - Check current rate limit status");
    println!();
    println!("Server running on http://localhost:3000");
    println!("Press Ctrl+C to stop");

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn strict_endpoint(headers: HeaderMap) -> Json<ApiResponse> {
    let rate_limit_info = extract_rate_limit_info(&headers);

    Json(ApiResponse {
        message: "This endpoint has strict rate limiting (5 requests per minute)".to_string(),
        remaining_requests: rate_limit_info.as_ref().map(|info| info.remaining),
        rate_limit_info,
    })
}

async fn moderate_endpoint(headers: HeaderMap) -> Json<ApiResponse> {
    let rate_limit_info = extract_rate_limit_info(&headers);

    Json(ApiResponse {
        message: "This endpoint has moderate rate limiting (20 requests per minute)".to_string(),
        remaining_requests: rate_limit_info.as_ref().map(|info| info.remaining),
        rate_limit_info,
    })
}

async fn login_endpoint(
    State(state): State<AppState>,
    _headers: HeaderMap,
    Json(login_req): Json<LoginRequest>,
) -> axum::response::Response {
    // IMPORTANT: The client must send the X-Login-Email header for rate limiting by email to work
    println!(
        "Login request email: {:?}, password: {:?}",
        login_req.email, login_req.password
    );

    // Create rate limiting configuration for login
    let login_config = BarnacleConfig {
        max_requests: 3,
        window: Duration::from_secs(300), // 5 minutes
        backoff: None,
        reset_on_success: true,
    };

    // First, validate the password
    if login_req.password == "correct_password" {
        // Reset rate limit on successful login
        let key = BarnacleKey::Email(login_req.email.clone());
        if let Err(_) = state.store.reset(&key).await {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }

        Json(LoginResponse {
            message: "Login successful! Rate limit reset.".to_string(),
            api_key: "fake_api_key_12345".to_string(),
            remaining_requests: Some(3), // Reset to maximum
            rate_limit_info: Some(RateLimitInfo {
                remaining: 3,
                limit: 3,
                reset_after: None,
            }),
        })
        .into_response()
    } else {
        // Login failed! Apply rate limiting only now
        let key = BarnacleKey::Email(login_req.email.clone());
        let result = state.store.increment(&key, &login_config).await;

        if !result.allowed {
            // Rate limit exceeded
            return axum::response::Response::builder()
                .status(429)
                .header(
                    "Retry-After",
                    result
                        .retry_after
                        .map(|d| d.as_secs().to_string())
                        .unwrap_or_else(|| "300".to_string()),
                )
                .header("X-RateLimit-Remaining", "0")
                .header("X-RateLimit-Limit", "3")
                .body(axum::body::Body::from(
                    "Too many failed login attempts. Try again later.",
                ))
                .unwrap();
        }

        println!(
            "Unauthorized login - remaining attempts: {}",
            result.remaining
        );
        StatusCode::UNAUTHORIZED.into_response()
    }
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

async fn status_endpoint(headers: HeaderMap) -> Json<ApiResponse> {
    let rate_limit_info = extract_rate_limit_info(&headers);

    Json(ApiResponse {
        message: "Rate limiter is working! Check the response headers for rate limit info."
            .to_string(),
        remaining_requests: rate_limit_info.as_ref().map(|info| info.remaining),
        rate_limit_info,
    })
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

#[derive(Serialize, Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Serialize, Deserialize)]
struct LoginResponse {
    message: String,
    api_key: String,
    remaining_requests: Option<u32>,
    rate_limit_info: Option<RateLimitInfo>,
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
