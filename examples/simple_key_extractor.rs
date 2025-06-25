use async_trait::async_trait;
use axum::{Router, http::StatusCode, response::Json, routing::post};
use barnacle::{
    BarnacleStore, KeyExtractable, create_generic_rate_limit_layer,
    types::{BarnacleConfig, BarnacleKey, BarnacleResult},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::net::TcpListener;

// Simple in-memory store for testing
#[derive(Debug)]
struct MemoryStore {
    data: Arc<Mutex<HashMap<String, (u32, u64)>>>, // key -> (count, expiry_timestamp)
}

impl MemoryStore {
    fn new() -> Self {
        Self {
            data: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn get_key_string(&self, key: &BarnacleKey) -> String {
        match key {
            BarnacleKey::Email(email) => format!("email:{}", email),
            BarnacleKey::ApiKey(api_key) => format!("api_key:{}", api_key),
            BarnacleKey::Ip(ip) => format!("ip:{}", ip),
        }
    }

    fn current_timestamp() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }
}

#[async_trait]
impl BarnacleStore for MemoryStore {
    async fn increment(&self, key: &BarnacleKey, config: &BarnacleConfig) -> BarnacleResult {
        let key_str = self.get_key_string(key);
        let current_time = Self::current_timestamp();
        let window_secs = config.window.as_secs();

        let mut data = self.data.lock().unwrap();

        // Clean up expired entries
        data.retain(|_, (_, expiry)| *expiry > current_time);

        let (count, expiry) = data
            .entry(key_str.clone())
            .or_insert((0, current_time + window_secs));

        // Check if window has expired
        if *expiry <= current_time {
            *count = 0;
            *expiry = current_time + window_secs;
        }

        // Check rate limit
        if *count >= config.max_requests {
            return BarnacleResult {
                allowed: false,
                remaining: 0,
                retry_after: Some(Duration::from_secs(*expiry - current_time)),
            };
        }

        // Increment counter
        *count += 1;

        BarnacleResult {
            allowed: true,
            remaining: config.max_requests - *count,
            retry_after: None,
        }
    }

    async fn reset(&self, key: &BarnacleKey) -> anyhow::Result<()> {
        let key_str = self.get_key_string(key);
        let mut data = self.data.lock().unwrap();
        data.remove(&key_str);
        Ok(())
    }
}

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
    println!("✅ Processing login for: {}", payload.email);

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
        "✅ Processing registration for: {} ({})",
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
        "✅ Processing API request with key: {} - action: {}",
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
    println!("🚀 Starting Generic Key Extractor Example Server...");

    // Initialize memory store
    let memory_store = Arc::new(MemoryStore::new());

    // Create different rate limit configurations
    let strict_config = BarnacleConfig {
        max_requests: 3,
        window: Duration::from_secs(60),
        backoff: None,
        reset_on_success: false,
    };

    let moderate_config = BarnacleConfig {
        max_requests: 5,
        window: Duration::from_secs(30),
        backoff: None,
        reset_on_success: false,
    };

    let lenient_config = BarnacleConfig {
        max_requests: 10,
        window: Duration::from_secs(60),
        backoff: None,
        reset_on_success: false,
    };

    // Create the router with different rate limits for different endpoints
    let app = Router::new()
        .route("/login", post(login_handler))
        .layer(create_generic_rate_limit_layer::<LoginPayload, _>(
            memory_store.clone(),
            strict_config.clone(),
        ))
        .route("/register", post(register_handler))
        .layer(create_generic_rate_limit_layer::<RegisterPayload, _>(
            memory_store.clone(),
            moderate_config.clone(),
        ))
        .route("/api", post(api_handler))
        .layer(create_generic_rate_limit_layer::<ApiRequestPayload, _>(
            memory_store.clone(),
            lenient_config.clone(),
        ));

    println!("📋 Server configuration:");
    println!(
        "  ├─ Login endpoint (/login): {} requests per {} seconds (by email)",
        strict_config.max_requests,
        strict_config.window.as_secs()
    );
    println!(
        "  ├─ Register endpoint (/register): {} requests per {} seconds (by email)",
        moderate_config.max_requests,
        moderate_config.window.as_secs()
    );
    println!(
        "  └─ API endpoint (/api): {} requests per {} seconds (by API key)",
        lenient_config.max_requests,
        lenient_config.window.as_secs()
    );
    println!();
    println!("🧪 Example requests to test rate limiting:");
    println!();
    println!(
        "  📧 Login (strict: {} req/min):",
        strict_config.max_requests
    );
    println!(
        "     curl -X POST http://localhost:3000/login -H 'Content-Type: application/json' \\"
    );
    println!("          -d '{{\"email\":\"user@example.com\",\"password\":\"secret\"}}'");
    println!();
    println!(
        "  📝 Register (moderate: {} req/{}s):",
        moderate_config.max_requests,
        moderate_config.window.as_secs()
    );
    println!(
        "     curl -X POST http://localhost:3000/register -H 'Content-Type: application/json' \\"
    );
    println!(
        "          -d '{{\"email\":\"user@example.com\",\"username\":\"user123\",\"password\":\"secret\"}}'"
    );
    println!();
    println!(
        "  🔑 API (lenient: {} req/min):",
        lenient_config.max_requests
    );
    println!("     curl -X POST http://localhost:3000/api -H 'Content-Type: application/json' \\");
    println!(
        "          -d '{{\"api_key\":\"key123\",\"action\":\"get_data\",\"data\":{{\"query\":\"test\"}}}}'"
    );
    println!();
    println!("💡 Tips for testing:");
    println!("  - Use the same email for login/register to see shared rate limiting");
    println!("  - Use different API keys to see separate rate limiting");
    println!("  - Try exceeding the limits to see rate limiting in action");
    println!();

    // Start the server
    let listener = TcpListener::bind("0.0.0.0:3000").await?;
    println!("🌐 Server running on http://localhost:3000");
    println!("   Press Ctrl+C to stop");

    axum::serve(listener, app).await?;

    Ok(())
}
