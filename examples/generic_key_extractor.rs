use axum::{Router, http::StatusCode, response::Json, routing::post};
use barnacle::{
    KeyExtractable, RedisBarnacleStore, create_generic_rate_limit_layer,
    types::{BarnacleConfig, BarnacleKey},
};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};
use tokio::net::TcpListener;

// Example payload types
#[derive(Deserialize, Debug)]
pub struct LoginPayload {
    pub email: String,
    pub password: String,
}

impl KeyExtractable for LoginPayload {
    fn extract_key(&self) -> BarnacleKey {
        BarnacleKey::Email(self.email.clone())
    }
}

#[derive(Deserialize, Debug)]
pub struct RegisterPayload {
    pub email: String,
    pub username: String,
    pub password: String,
}

impl KeyExtractable for RegisterPayload {
    fn extract_key(&self) -> BarnacleKey {
        BarnacleKey::Email(self.email.clone())
    }
}

#[derive(Deserialize, Debug)]
pub struct ApiRequestPayload {
    pub api_key: String,
    pub action: String,
    pub data: serde_json::Value,
}

impl KeyExtractable for ApiRequestPayload {
    fn extract_key(&self) -> BarnacleKey {
        BarnacleKey::ApiKey(self.api_key.clone())
    }
}

#[derive(Serialize)]
pub struct ApiResponse {
    pub message: String,
    pub data: Option<serde_json::Value>,
}

// Handler functions
pub async fn login_handler(
    axum::Json(payload): axum::Json<LoginPayload>,
) -> Result<Json<ApiResponse>, StatusCode> {
    println!("Processing login for: {}", payload.email);

    // Simulate some processing
    tokio::time::sleep(Duration::from_millis(100)).await;

    Ok(Json(ApiResponse {
        message: "Login processed successfully".to_string(),
        data: Some(serde_json::json!({
            "email": payload.email,
            "status": "authenticated"
        })),
    }))
}

pub async fn register_handler(
    axum::Json(payload): axum::Json<RegisterPayload>,
) -> Result<Json<ApiResponse>, StatusCode> {
    println!(
        "Processing registration for: {} ({})",
        payload.email, payload.username
    );

    // Simulate some processing
    tokio::time::sleep(Duration::from_millis(150)).await;

    Ok(Json(ApiResponse {
        message: "Registration processed successfully".to_string(),
        data: Some(serde_json::json!({
            "email": payload.email,
            "username": payload.username,
            "status": "registered"
        })),
    }))
}

pub async fn api_handler(
    axum::Json(payload): axum::Json<ApiRequestPayload>,
) -> Result<Json<ApiResponse>, StatusCode> {
    println!(
        "Processing API request with key: {} - action: {}",
        payload.api_key, payload.action
    );

    // Simulate some processing
    tokio::time::sleep(Duration::from_millis(50)).await;

    Ok(Json(ApiResponse {
        message: format!("API action '{}' processed successfully", payload.action),
        data: Some(payload.data),
    }))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting Generic Key Extractor Example Server...");

    // Initialize Redis store
    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
    let redis_client = Arc::new(redis::Client::open(redis_url)?);
    let redis_store = Arc::new(RedisBarnacleStore::new(redis_client));

    // Create different rate limit configurations
    let strict_config = BarnacleConfig {
        max_requests: 3,
        window: Duration::from_secs(60),
        backoff: None,
        reset_on_success: false,
    };

    let moderate_config = BarnacleConfig {
        max_requests: 10,
        window: Duration::from_secs(60),
        backoff: None,
        reset_on_success: false,
    };

    let lenient_config = BarnacleConfig {
        max_requests: 100,
        window: Duration::from_secs(60),
        backoff: None,
        reset_on_success: false,
    };

    // Create the router with different rate limits for different endpoints
    let app = Router::new()
        .route("/login", post(login_handler))
        .layer(create_generic_rate_limit_layer::<LoginPayload, _>(
            redis_store.clone(),
            strict_config.clone(),
        ))
        .route("/register", post(register_handler))
        .layer(create_generic_rate_limit_layer::<RegisterPayload, _>(
            redis_store.clone(),
            moderate_config.clone(),
        ))
        .route("/api", post(api_handler))
        .layer(create_generic_rate_limit_layer::<ApiRequestPayload, _>(
            redis_store.clone(),
            lenient_config.clone(),
        ));

    println!("Server configuration:");
    println!(
        "  - Login endpoint (/login): {} requests per minute (by email)",
        strict_config.max_requests
    );
    println!(
        "  - Register endpoint (/register): {} requests per minute (by email)",
        moderate_config.max_requests
    );
    println!(
        "  - API endpoint (/api): {} requests per minute (by API key)",
        lenient_config.max_requests
    );
    println!();
    println!("Example requests:");
    println!(
        "  Login:    curl -X POST http://localhost:3000/login -H 'Content-Type: application/json' -d '{{\"email\":\"user@example.com\",\"password\":\"secret\"}}'"
    );
    println!(
        "  Register: curl -X POST http://localhost:3000/register -H 'Content-Type: application/json' -d '{{\"email\":\"user@example.com\",\"username\":\"user123\",\"password\":\"secret\"}}'"
    );
    println!(
        "  API:      curl -X POST http://localhost:3000/api -H 'Content-Type: application/json' -d '{{\"api_key\":\"key123\",\"action\":\"get_data\",\"data\":{{\"query\":\"test\"}}}}'"
    );
    println!();

    // Start the server
    let listener = TcpListener::bind("0.0.0.0:3000").await?;
    println!("🚀 Server running on http://localhost:3000");

    axum::serve(listener, app).await?;

    Ok(())
}
