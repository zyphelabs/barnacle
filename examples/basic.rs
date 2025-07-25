use std::time::Duration;

use axum::{
    extract::State,
    http::{request::Parts, HeaderMap, StatusCode},
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use barnacle_rs::{BarnacleConfig, BarnacleContext, BarnacleError, BarnacleKey, BarnacleLayer, BarnacleStore, KeyExtractable, RedisBarnacleStore, ResetOnSuccess};
use serde::{Deserialize, Serialize};

impl KeyExtractable for LoginRequest {
    fn extract_key(&self, _request_parts: &Parts) -> BarnacleKey {
        BarnacleKey::Email(self.email.clone())
    }
}

pub fn init_tracing() {
    use tracing_subscriber::fmt::format::FmtSpan;

    let log_env_filter = std::env::var("LOG_LEVEL").unwrap_or_else(|_| "debug".into());

    tracing_subscriber::fmt()
        .with_env_filter(log_env_filter)
        .with_target(true)
        .with_level(true)
        .with_span_events(FmtSpan::CLOSE)
        .pretty()
        .init();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();

    // Create Redis store with connection pooling
    let store = RedisBarnacleStore::from_url("redis://127.0.0.1:6379")
        .map_err(|e| format!("Failed to create Redis store with connection pool: {}", e))?;

    let state = AppState {
        store: store.clone(),
    };

    // Different rate limiting configurations
    let login_config = BarnacleConfig {
        max_requests: 3,
        window: Duration::from_secs(60),
        reset_on_success: ResetOnSuccess::Yes(None), // Reset on 2xx status codes
    };

    let strict_config = BarnacleConfig {
        max_requests: 5,
        window: Duration::from_secs(60),
        reset_on_success: ResetOnSuccess::Not,
    };

    let moderate_config = BarnacleConfig {
        max_requests: 10,
        window: Duration::from_secs(60),
        reset_on_success: ResetOnSuccess::Not,
    };

    // Create different middleware layers for different endpoints
    let login_layer: BarnacleLayer<LoginRequest, _, (), BarnacleError, ()> = BarnacleLayer::builder().with_store(store.clone()).with_config(login_config.clone()).build().unwrap();

    let strict_layer: BarnacleLayer<(), _, (), BarnacleError, ()> = BarnacleLayer::builder().with_store(store.clone()).with_config(strict_config).build().unwrap();
    let moderate_layer: BarnacleLayer<(), _, (), BarnacleError, ()> = BarnacleLayer::builder().with_store(store.clone()).with_config(moderate_config).build().unwrap();

    let app = Router::new()
        .route("/api/strict", get(strict_endpoint).layer(strict_layer))
        .route(
            "/api/moderate",
            get(moderate_endpoint).layer(moderate_layer),
        )
        .route("/api/login", post(login_endpoint).layer(login_layer))
        .route("/api/reset/{:key_type}/{:value}", post(reset_rate_limit))
        .route("/api/status", get(status_endpoint))
        .with_state(state);

    tracing::info!("🚀 Barnacle Rate Limiter Demo Server");
    tracing::info!("Server running on http://localhost:3000");
    tracing::info!("Press Ctrl+C to stop");

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    axum::serve(listener, app).await?;

    Ok(())
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
        message: "This endpoint has moderate rate limiting (10 requests per minute)".to_string(),
        remaining_requests: rate_limit_info.as_ref().map(|info| info.remaining),
        rate_limit_info,
    })
}

async fn login_endpoint(
    State(_state): State<AppState>,
    _headers: HeaderMap,
    Json(login_req): Json<LoginRequest>,
) -> axum::response::Response {
    // IMPORTANT: The client must send the X-Login-Email header for rate limiting by email to work
    tracing::debug!(
        "Login request email: {:?}, password: {:?}",
        login_req.email,
        login_req.password
    );

    // First, validate the password
    if login_req.password == "correct_password" {
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

    // Create context with empty path/method for reset endpoint
    let context = BarnacleContext {
        key,
        path: "/reset".to_string(),
        method: "POST".to_string(),
    };

    match state.store.reset(&context).await {
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
    store: RedisBarnacleStore,
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
