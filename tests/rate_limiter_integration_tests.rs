use std::time::Duration;

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
    routing::{get, post},
};
use barnacle_rs::{
    BarnacleConfig, BarnacleKey, BarnacleStore, KeyExtractable, RedisBarnacleStore, ResetOnSuccess,
    create_barnacle_layer, create_barnacle_layer_for_payload,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::time::sleep;

// Test application setup - mirrors basic.rs example
impl KeyExtractable for LoginRequest {
    fn extract_key(&self) -> BarnacleKey {
        BarnacleKey::Email(self.email.clone())
    }
}

#[derive(Clone)]
struct AppState {
    store: RedisBarnacleStore,
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

async fn create_test_app() -> (Router, RedisBarnacleStore) {
    // Create Redis store for testing (uses test Redis instance)
    let store = RedisBarnacleStore::from_url("redis://127.0.0.1:6379")
        .await
        .expect("Failed to create Redis store for testing");

    let state = AppState {
        store: store.clone(),
    };

    // Rate limiting configurations (same as basic.rs)
    let login_config = BarnacleConfig {
        max_requests: 4,
        window: Duration::from_secs(300), // 5 minutes for login
        reset_on_success: ResetOnSuccess::Yes(None),
    };

    let strict_config = BarnacleConfig {
        max_requests: 5,
        window: Duration::from_secs(60), // 1 minute
        reset_on_success: ResetOnSuccess::Not,
    };

    let moderate_config = BarnacleConfig {
        max_requests: 20,                // Updated to match shell script comment
        window: Duration::from_secs(60), // 1 minute
        reset_on_success: ResetOnSuccess::Not,
    };

    let login_limiter =
        create_barnacle_layer_for_payload::<LoginRequest>(store.clone(), login_config);
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

    (app, store)
}

// Handler functions (same as basic.rs)
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
    State(_state): State<AppState>,
    _headers: HeaderMap,
    Json(login_req): Json<LoginRequest>,
) -> axum::response::Response {
    if login_req.password == "correct_password" {
        Json(json!("Login successful! Rate limit reset.")).into_response()
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

// Test helper functions
async fn start_test_server() -> String {
    let (app, _store) = create_test_app().await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind test server");

    let addr = listener.local_addr().expect("Failed to get server address");
    let base_url = format!("http://{}", addr);

    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("Test server failed");
    });

    // Give the server a moment to start
    sleep(Duration::from_millis(100)).await;

    base_url
}

// Test 1: Basic Rate Limiting (5 requests per minute), this test will fail second time cargo test is run
#[tokio::test]
async fn test_basic_rate_limiting() {
    let base_url = start_test_server().await;
    let client = reqwest::Client::new();

    // Make 6 requests to /api/strict endpoint
    let mut responses = Vec::new();
    for i in 1..=6 {
        let response = client
            .get(&format!("{}/api/strict", base_url))
            .send()
            .await
            .expect(&format!("Request {} failed", i));

        println!("Request {} status: {:?}", i, response.status());

        responses.push(response.status());
        sleep(Duration::from_millis(100)).await;
    }

    // First 5 requests should succeed (200), 6th should fail (429)
    for (i, status) in responses.iter().enumerate() {
        if i < 5 {
            assert_eq!(
                *status,
                reqwest::StatusCode::OK,
                "Request {} should succeed",
                i + 1
            );
        } else {
            assert_eq!(
                *status,
                reqwest::StatusCode::TOO_MANY_REQUESTS,
                "Request {} should be rate limited",
                i + 1
            );
        }
    }
}

// Test 2: Different Rate Limits, this should fail if cargo test runs 4 times.
#[tokio::test]
async fn test_different_rate_limits() {
    let base_url = start_test_server().await;
    let client = reqwest::Client::new();

    // Test moderate endpoint (20 requests per minute)
    // Make 6 requests to /api/moderate endpoint
    let mut responses = Vec::new();
    for i in 1..=6 {
        let response = client
            .get(&format!("{}/api/moderate", base_url))
            .send()
            .await
            .expect(&format!("Request {} failed", i));

        println!("Moderate Request {} status: {:?}", i, response.status());

        responses.push(response.status());
        sleep(Duration::from_millis(100)).await;
    }

    // All 6 requests should succeed (200) since moderate allows 20 requests per minute
    for (i, status) in responses.iter().enumerate() {
        assert_eq!(
            *status,
            reqwest::StatusCode::OK,
            "Moderate request {} should succeed",
            i + 1
        );
    }
}

// Test 3: Login Rate Limiting with Different Emails
#[tokio::test]
async fn test_login_rate_limiting_different_emails() {
    let base_url = start_test_server().await;
    let client = reqwest::Client::new();

    // Reset the rate limit for user1@example.com, reseting for tests always pass
    let reset_response = client
        .post(&format!("{}/api/reset/email/user1@example.com", base_url))
        .send()
        .await
        .expect("Reset request failed");

    println!("Reset response status: {:?}", reset_response.status());

    // Reset should succeed (200)
    assert_eq!(
        reset_response.status(),
        reqwest::StatusCode::OK,
        "Reset should succeed"
    );

    let mut responses = Vec::new();
    let login_data = json!({
        "email": "user1@example.com",
        "password": "wrong_password"
    });
    for i in 1..=5 {
        let response = client
            .post(&format!("{}/api/login", base_url))
            .json(&login_data)
            .send()
            .await
            .expect(&format!("Login request {} failed", i));

        println!("Failed login {} status: {:?}", i, response.status());

        responses.push(response.status());
        sleep(Duration::from_millis(100)).await;
    }

    // First 4 attempts should fail per 401, last one should fail per 429
    for (i, status) in responses.iter().enumerate() {
        if i < 4 {
            assert_eq!(
                *status,
                reqwest::StatusCode::UNAUTHORIZED,
                "Failed login {} should return 401",
                i + 1
            );
        } else {
            assert_eq!(
                *status,
                reqwest::StatusCode::TOO_MANY_REQUESTS,
                "5th failed login should be rate limited"
            );
        }
    }

    // Now try a successful login with a different email - should work
    let login_data_user2 = json!({
        "email": "user2@example.com",
        "password": "correct_password"
    });

    let response = client
        .post(&format!("{}/api/login", base_url))
        .json(&login_data_user2)
        .send()
        .await
        .expect("Successful login request failed");

    // The successful login should work (200)
    assert_eq!(
        response.status(),
        reqwest::StatusCode::OK,
        "Successful login with different email should work"
    );

    // Verify the response contains the success message
    let response_text = response.text().await.expect("Failed to read response text");
    assert!(
        response_text.contains("Login successful"),
        "Response should contain success message"
    );
}
