use axum::{Json, Router, routing::get};
use barnacle::RedisBarnacleStore;
use barnacle::{BarnacleConfig, RedisApiKeyStore, create_api_key_layer};
use deadpool_redis::Config as RedisConfig;
use serde_json::json;
use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    // Redis pool setup
    let redis_cfg = RedisConfig::from_url("redis://127.0.0.1/");
    let pool = redis_cfg.create_pool(None).unwrap();

    // Multiple API keys with different rate limit configurations
    let api_keys = vec![
        (
            "valid-key-123",
            BarnacleConfig {
                max_requests: 3,
                window: std::time::Duration::from_secs(6), // 6 seconds for faster testing
                ..Default::default()
            },
        ),
        (
            "premium-key-456",
            BarnacleConfig {
                max_requests: 10,
                window: std::time::Duration::from_secs(6), // 6 seconds for faster testing
                ..Default::default()
            },
        ),
        (
            "basic-key-789",
            BarnacleConfig {
                max_requests: 1,
                window: std::time::Duration::from_secs(6), // 6 seconds for faster testing
                ..Default::default()
            },
        ),
    ];

    // Insert all API keys and their configs into Redis for testing
    {
        let mut conn = pool.get().await.expect("Failed to get Redis connection");

        for (api_key, rate_limit) in &api_keys {
            let key = format!("barnacle:api_keys:{}", api_key);
            let config_key = format!("barnacle:api_keys:config:{}", api_key);

            // Set API key as valid
            let _: () = deadpool_redis::redis::cmd("SET")
                .arg(&key)
                .arg(1)
                .query_async(&mut conn)
                .await
                .expect(&format!("Failed to set API key: {}", api_key));

            // Set rate limit configuration
            let config_json = serde_json::to_string(&rate_limit).unwrap();
            let _: () = deadpool_redis::redis::cmd("SET")
                .arg(&config_key)
                .arg(&config_json)
                .query_async(&mut conn)
                .await
                .expect(&format!("Failed to set config for API key: {}", api_key));
        }

        println!("✅ Configured {} API keys in Redis:", api_keys.len());
        for (api_key, config) in &api_keys {
            println!(
                "   - {}: {} requests per {} seconds",
                api_key,
                config.max_requests,
                config.window.as_secs()
            );
        }
    }

    let api_key_store = RedisApiKeyStore::new(pool.clone(), BarnacleConfig::default());
    let rate_limit_store = RedisBarnacleStore::new(pool);
    let api_key_layer = create_api_key_layer(api_key_store, rate_limit_store);

    // Test endpoint
    let app = Router::new()
        .route("/test", get(test_handler))
        .layer(api_key_layer);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    println!("🚀 Test server running at http://{}", addr);
    println!("📋 Available API keys for testing:");
    for (api_key, config) in &api_keys {
        println!(
            "   - {} ({} req/{}s)",
            api_key,
            config.max_requests,
            config.window.as_secs()
        );
    }
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
