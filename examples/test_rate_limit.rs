use axum::{
    Router,
    response::{IntoResponse, Json},
    routing::get,
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .with_target(false)
        .compact()
        .init();

    info!("🧪 Starting Rate Limit Test Server...");
    info!("====================================");

    // Create API key configurations with VERY LOW limits for easy testing
    let static_config = StaticApiKeyConfig::new(BarnacleConfig {
        max_requests: 2,                 // Default: apenas 2 requests
        window: Duration::from_secs(30), // Em 30 segundos
        reset_on_success: ResetOnSuccess::Not,
    })
    .with_key_config(
        "test_low_limit".to_string(),
        BarnacleConfig {
            max_requests: 3,                 // Apenas 3 requests
            window: Duration::from_secs(20), // Em 20 segundos
            reset_on_success: ResetOnSuccess::Not,
        },
    )
    .with_key_config(
        "test_very_low".to_string(),
        BarnacleConfig {
            max_requests: 1,                 // Apenas 1 request!
            window: Duration::from_secs(10), // Em 10 segundos
            reset_on_success: ResetOnSuccess::Not,
        },
    )
    .with_key_config(
        "test_reset_on_success".to_string(),
        BarnacleConfig {
            max_requests: 2,
            window: Duration::from_secs(60),
            reset_on_success: ResetOnSuccess::Yes(None), // Reset no sucesso
        },
    );

    let static_store = Arc::new(StaticApiKeyStore::new(static_config));

    // Use simple in-memory rate limiter for testing
    let rate_limiter = Arc::new(SimpleRateLimiter::new());

    // Configure middleware to require API keys
    let api_key_config = ApiKeyMiddlewareConfig {
        header_name: "x-api-key".to_string(),
        default_rate_limit: BarnacleConfig {
            max_requests: 2,
            window: Duration::from_secs(30),
            reset_on_success: ResetOnSuccess::Not,
        },
        require_api_key: true, // API key obrigatória
    };

    // Create middleware
    let middleware = create_api_key_layer_with_config(static_store, rate_limiter, api_key_config);

    // Create the application
    let app = Router::new()
        .route("/test", get(test_handler))
        .layer(middleware)
        .route("/health", get(health_handler));

    info!("🎯 Server started on http://localhost:3000");
    info!("");
    info!("🔑 API Keys configuradas para teste:");
    info!("  test_low_limit     → 3 requests/20s");
    info!("  test_very_low      → 1 request/10s");
    info!("  test_reset_on_success → 2 requests/60s (reset on 2xx)");
    info!("");
    info!("🧪 Comandos de teste:");
    info!("  # Test API key denial:");
    info!("  curl http://localhost:3000/test  # Sem API key → 401");
    info!("  curl -H 'x-api-key: invalid' http://localhost:3000/test  # Key inválida → 401");
    info!("");
    info!("  # Test rate limiting:");
    info!("  curl -H 'x-api-key: test_very_low' http://localhost:3000/test  # 1º: OK");
    info!("  curl -H 'x-api-key: test_very_low' http://localhost:3000/test  # 2º: 429");
    info!("");
    info!("  # Teste automatizado:");
    info!("  ./test_api_key_denial.sh");
    info!("  ./test_rate_limit_denial.sh");

    let listener = TcpListener::bind("127.0.0.1:3000").await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn test_handler() -> impl IntoResponse {
    Json(json!({
        "message": "Request aceito!",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "status": "success"
    }))
}

async fn health_handler() -> impl IntoResponse {
    Json(json!({
        "status": "healthy",
        "timestamp": chrono::Utc::now().to_rfc3339()
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
