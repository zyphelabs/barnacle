use std::sync::Arc;
use std::time::Duration;

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
    routing::{get, post},
};
use barnacle::{
    BarnacleConfig, BarnacleKey, BarnacleStore, KeyExtractable, RedisBarnacleStore, ResetOnSuccess,
    create_barnacle_layer, create_barnacle_layer_for_payload,
};
use serde::{Deserialize, Serialize};
use tracing;

impl KeyExtractable for LoginRequest {
    fn extract_key(&self) -> BarnacleKey {
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
        .await
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
    let login_limiter =
        create_barnacle_layer_for_payload::<LoginRequest>(store.clone(), login_config.clone());

    let strict_limiter = create_barnacle_layer(store.clone(), strict_config);
    let moderate_limiter = create_barnacle_layer(store.clone(), moderate_config);

    let app = Router::new()
        .route("/api/strict", get(strict_endpoint).layer(strict_limiter))
        .route(
            "/api/moderate",
            get(moderate_endpoint).layer(moderate_limiter),
        )
        .route("/api/login", post(login_endpoint).layer(login_limiter))
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
