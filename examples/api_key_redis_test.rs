use axum::{http::request::Parts, routing::get, Json, Router};
use barnacle_rs::{ApiKeyConfig, BarnacleConfig, BarnacleError, BarnacleLayer, RedisBarnacleStore};
use serde_json::json;
use std::{net::SocketAddr, sync::Arc};

#[tokio::main]
async fn main() {
    // Redis pool setup
    let redis_cfg = deadpool_redis::Config::from_url("redis://127.0.0.1/");
    let pool = redis_cfg.create_pool(None).unwrap();
    let store = RedisBarnacleStore::new(pool.clone());
    let config = BarnacleConfig {
        max_requests: 3,
        window: std::time::Duration::from_secs(6),
        ..Default::default()
    };
    let api_key_validator = |api_key: String, _api_key_config: ApiKeyConfig, _parts: Arc<Parts>, _state: ()| async move {
        if api_key != "valid-key-123" {
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

    // Test endpoint
    let app = Router::new()
        .route("/test", get(test_handler))
        .layer(middleware);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    println!("🚀 Test server running at http://{}", addr);
    println!("📋 Available API keys for testing:");
    println!("   - valid-key-123 (3 req/6s)");
    println!("   - invalid-key-xyz (invalid)");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn test_handler() -> Json<serde_json::Value> {
    Json(json!({
        "message": "API key and rate limit test successful",
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}
