use barnacle::{BarnacleConfig, ResetOnSuccess};
use deadpool_redis::redis::AsyncCommands;
use deadpool_redis::{Config, Pool, Runtime};
use serde_json;
use std::time::Duration;
use tracing::{Level, info};
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .with_target(false)
        .compact()
        .init();

    info!("Setting up Redis with API keys and configurations...");

    // Connect to Redis
    let redis_config = Config::from_url("redis://localhost:6379");
    let pool = redis_config.create_pool(Some(Runtime::Tokio1))?;

    let mut conn = pool.get().await?;

    // Define API keys and their configurations
    let api_keys = vec![
        (
            "user_api_key_123",
            BarnacleConfig {
                max_requests: 100,
                window: Duration::from_secs(60),
                reset_on_success: ResetOnSuccess::Not,
            },
        ),
        (
            "premium_key_456",
            BarnacleConfig {
                max_requests: 1000,
                window: Duration::from_secs(60),
                reset_on_success: ResetOnSuccess::Yes(None), // Reset on 2xx status codes
            },
        ),
        (
            "limited_key_789",
            BarnacleConfig {
                max_requests: 10,
                window: Duration::from_secs(60),
                reset_on_success: ResetOnSuccess::Not,
            },
        ),
        (
            "enterprise_key_999",
            BarnacleConfig {
                max_requests: 10000,
                window: Duration::from_secs(3600), // 1 hour window
                reset_on_success: ResetOnSuccess::Yes(Some(vec![200, 201, 202])), // Reset only on specific success codes
            },
        ),
    ];

    // Setup API keys in Redis
    for (api_key, config) in api_keys {
        let key_path = format!("barnacle:api_keys:{}", api_key);
        let config_path = format!("barnacle:api_keys:config:{}", api_key);

        // Set the key existence flag
        let _: () = conn.set(&key_path, "1").await?;

        // Set the configuration as JSON
        let config_json = serde_json::to_string(&config)?;
        let _: () = conn.set(&config_path, &config_json).await?;

        info!(
            "✓ Added API key: {} with config: max_requests={}, window={}s",
            api_key,
            config.max_requests,
            config.window.as_secs()
        );
    }

    // Set up some sample data for demonstration
    info!("Setting up sample rate limit data...");

    // Add some initial rate limit data for demonstration
    let sample_keys = vec![
        "barnacle:api_key:user_api_key_123",
        "barnacle:api_key:premium_key_456",
    ];

    for key in sample_keys {
        let _: () = conn.set_ex(key, 5, 55).await?; // 5 requests used, expires in 55 seconds
        info!("✓ Set sample rate limit data for: {}", key);
    }

    info!("✓ Redis setup complete!");
    info!("");
    info!("You can now test the API key middleware with:");
    info!("  cargo run --example api_key_example");
    info!("");
    info!("Or test with curl:");
    info!("  curl -H 'x-api-key: user_api_key_123' http://localhost:3000/api/protected");
    info!("  curl -H 'x-api-key: premium_key_456' http://localhost:3000/api/protected");
    info!("  curl -H 'x-api-key: invalid_key' http://localhost:3000/api/protected");
    info!("");
    info!("To view the keys in Redis:");
    info!("  redis-cli KEYS 'barnacle:api_keys:*'");
    info!("  redis-cli GET 'barnacle:api_keys:config:user_api_key_123'");

    Ok(())
}

/// Helper function to clear all barnacle keys from Redis
#[allow(dead_code)]
async fn clear_redis_keys(pool: &Pool) -> Result<(), Box<dyn std::error::Error>> {
    let mut conn = pool.get().await?;

    // Get all barnacle keys
    let keys: Vec<String> = conn.keys("barnacle:*").await?;

    if !keys.is_empty() {
        // Delete all keys
        let _: () = conn.del(keys.clone()).await?;
        info!("Cleared {} keys from Redis", keys.len());
    } else {
        info!("No barnacle keys found in Redis");
    }

    Ok(())
}
