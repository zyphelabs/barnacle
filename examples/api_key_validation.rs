use axum::{
    Router,
    extract::State,
    response::{IntoResponse, Json},
    routing::{get, post},
};
use barnacle::{
    ApiKeyMiddlewareConfig, BarnacleConfig, BarnacleKey, BarnacleResult, BarnacleStore,
    ResetOnSuccess, StaticApiKeyConfig, StaticApiKeyStore, create_api_key_layer_with_config,
};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tracing::{Level, info};
use tracing_subscriber;

// App state to hold the rate limiter for reset functionality
#[derive(Clone)]
struct AppState {
    rate_limiter: Arc<SimpleRateLimiter>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .with_target(false)
        .compact()
        .init();

    info!("🔐 API Key Validation & Rate Limiting Demo Server");
    info!("================================================");

    // Create API key configurations with different rate limits for testing
    let static_config = StaticApiKeyConfig::new(BarnacleConfig {
        max_requests: 5,                 // Default: 5 requests
        window: Duration::from_secs(60), // In 60 seconds
        reset_on_success: ResetOnSuccess::Not,
    })
    .with_key_config(
        "premium_key".to_string(),
        BarnacleConfig {
            max_requests: 10,                // Premium: 10 requests
            window: Duration::from_secs(60), // In 60 seconds
            reset_on_success: ResetOnSuccess::Not,
        },
    )
    .with_key_config(
        "basic_key".to_string(),
        BarnacleConfig {
            max_requests: 3,                 // Basic: 3 requests
            window: Duration::from_secs(60), // In 60 seconds
            reset_on_success: ResetOnSuccess::Not,
        },
    )
    .with_key_config(
        "strict_key".to_string(),
        BarnacleConfig {
            max_requests: 2,                 // Strict: 2 requests
            window: Duration::from_secs(30), // In 30 seconds
            reset_on_success: ResetOnSuccess::Not,
        },
    );

    let static_store = Arc::new(StaticApiKeyStore::new(static_config));

    // Use simple in-memory rate limiter for testing
    let rate_limiter = Arc::new(SimpleRateLimiter::new());

    // Configure middleware to require API keys
    let api_key_config = ApiKeyMiddlewareConfig {
        header_name: "x-api-key".to_string(),
        default_rate_limit: BarnacleConfig {
            max_requests: 5,
            window: Duration::from_secs(60),
            reset_on_success: ResetOnSuccess::Not,
        },
        require_api_key: true, // API key required
    };

    // Create middleware
    let middleware =
        create_api_key_layer_with_config(static_store, rate_limiter.clone(), api_key_config);

    let state = AppState {
        rate_limiter: rate_limiter.clone(),
    };

    // Create the application
    let app = Router::new()
        .route("/api/data", get(data_handler))
        .route("/api/status", get(status_handler))
        .route("/api/reset/:api_key", post(reset_rate_limit))
        .layer(middleware)
        .route("/health", get(health_handler))
        .with_state(state);

    info!("🎯 Server started on http://localhost:3000");
    info!("");
    info!("🔑 API Keys configured for testing:");
    info!("  premium_key  → 10 requests/60s");
    info!("  basic_key    → 3 requests/60s");
    info!("  strict_key   → 2 requests/30s");
    info!("  (default)    → 5 requests/60s");
    info!("");
    info!("🧪 Test Scenarios:");
    info!("  1. Missing API key → 401 Unauthorized");
    info!("  2. Invalid API key → 401 Unauthorized");
    info!("  3. Rate limit exceeded → 429 Too Many Requests");
    info!("");
    info!("📝 Test Commands:");
    info!("  # Test missing API key:");
    info!("  curl http://localhost:3000/api/data");
    info!("");
    info!("  # Test invalid API key:");
    info!("  curl -H 'x-api-key: invalid_key' http://localhost:3000/api/data");
    info!("");
    info!("  # Test rate limiting:");
    info!("  curl -H 'x-api-key: strict_key' http://localhost:3000/api/data");
    info!("  curl -H 'x-api-key: strict_key' http://localhost:3000/api/data");
    info!("  curl -H 'x-api-key: strict_key' http://localhost:3000/api/data  # Should fail");
    info!("");
    info!("  # Reset rate limit for testing:");
    info!("  curl -X POST http://localhost:3000/api/reset/strict_key");
    info!("");
    info!("  # Run automated tests:");
    info!("  ./test_api_key_scenarios.sh");

    let listener = TcpListener::bind("127.0.0.1:3000").await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn data_handler() -> impl IntoResponse {
    Json(json!({
        "message": "Data endpoint accessed successfully!",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "status": "success",
        "data": {
            "id": 12345,
            "name": "Sample Data",
            "description": "This is protected data that requires a valid API key"
        }
    }))
}

async fn status_handler() -> impl IntoResponse {
    Json(json!({
        "message": "Status endpoint accessed successfully!",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "status": "success",
        "service": "API Key Validation Demo"
    }))
}

async fn reset_rate_limit(
    State(state): State<AppState>,
    axum::extract::Path(api_key): axum::extract::Path<String>,
) -> impl IntoResponse {
    let key = BarnacleKey::ApiKey(api_key.clone());

    match state.rate_limiter.reset(&key).await {
        Ok(_) => Json(json!({
            "message": format!("Rate limit reset for API key: {}", api_key),
            "status": "success",
            "timestamp": chrono::Utc::now().to_rfc3339()
        })),
        Err(_) => Json(json!({
            "message": format!("Failed to reset rate limit for API key: {}", api_key),
            "status": "error",
            "timestamp": chrono::Utc::now().to_rfc3339()
        })),
    }
}

async fn health_handler() -> impl IntoResponse {
    Json(json!({
        "status": "healthy",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "service": "API Key Validation Demo"
    }))
}

// Simple rate limiter with low limits for testing
use async_trait::async_trait;
use std::collections::HashMap;
use tokio::sync::RwLock;

struct SimpleRateLimiter {
    counts: Arc<RwLock<HashMap<String, (u32, std::time::Instant)>>>,
}

impl SimpleRateLimiter {
    fn new() -> Self {
        Self {
            counts: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn key_to_string(&self, key: &BarnacleKey) -> String {
        match key {
            BarnacleKey::ApiKey(k) => format!("api:{}", k),
            BarnacleKey::Email(e) => format!("email:{}", e),
            BarnacleKey::Ip(ip) => format!("ip:{}", ip),
            BarnacleKey::Custom(c) => format!("custom:{}", c),
        }
    }
}

#[async_trait]
impl BarnacleStore for SimpleRateLimiter {
    async fn increment(&self, key: &BarnacleKey, config: &BarnacleConfig) -> BarnacleResult {
        let key_str = self.key_to_string(key);
        let mut counts = self.counts.write().await;
        let now = std::time::Instant::now();

        // Clone to avoid borrow issues
        let current_entry = counts.get(&key_str).cloned();

        if let Some((count, last_reset)) = current_entry {
            if now.duration_since(last_reset) > config.window {
                // Window expired, reset
                counts.insert(key_str, (1, now));
                BarnacleResult {
                    allowed: true,
                    remaining: config.max_requests - 1,
                    retry_after: None,
                }
            } else if count >= config.max_requests {
                // Rate limit exceeded
                let retry_after = config.window - now.duration_since(last_reset);
                BarnacleResult {
                    allowed: false,
                    remaining: 0,
                    retry_after: Some(retry_after),
                }
            } else {
                // Increment count
                let new_count = count + 1;
                counts.insert(key_str, (new_count, last_reset));
                BarnacleResult {
                    allowed: true,
                    remaining: config.max_requests - new_count,
                    retry_after: None,
                }
            }
        } else {
            // First request
            counts.insert(key_str, (1, now));
            BarnacleResult {
                allowed: true,
                remaining: config.max_requests - 1,
                retry_after: None,
            }
        }
    }

    async fn reset(&self, key: &BarnacleKey) -> anyhow::Result<()> {
        let key_str = self.key_to_string(key);
        let mut counts = self.counts.write().await;
        counts.remove(&key_str);
        Ok(())
    }
}
