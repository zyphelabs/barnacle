use std::{sync::Arc, time::Duration};

use axum::{http::StatusCode, response::Json, routing::get, Router};
use deadpool_redis::Config as RedisConfig;
use serde_json::json;
use barnacle_rs::{BarnacleLayer, RedisBarnacleStore, ApiKeyConfig};
use std::sync::Once;
use uuid::Uuid;
use barnacle_rs::BarnacleError;
use axum::http::request::Parts;

static INIT: Once = Once::new();

fn init_tracing() {
    INIT.call_once(|| {
        tracing_subscriber::fmt::init();
    });
}

// Test constants
const VALID_KEY: &str = "valid-key-123";
const VALID_KEY_2: &str = "valid-key-456";
// In the test setup, ensure max_requests is at least 2 for rate limit tests
const RATE_LIMIT_VALID: u32 = 2;
const WINDOW_SECONDS: u64 = 6;

async fn cleanup_redis() {
    let redis_cfg = RedisConfig::from_url("redis://127.0.0.1/");
    let pool = redis_cfg
        .create_pool(None)
        .expect("Failed to create Redis pool");
    let mut conn = pool.get().await.expect("Failed to get Redis connection");

    // Clean up all barnacle keys
    let keys: Vec<String> = deadpool_redis::redis::cmd("KEYS")
        .arg("barnacle:*")
        .query_async(&mut conn)
        .await
        .unwrap_or_default();

    if !keys.is_empty() {
        let mut cmd = deadpool_redis::redis::cmd("DEL");
        for key in keys {
            cmd.arg(key);
        }
        let _: () = cmd.query_async(&mut conn).await.unwrap_or_default();
    }
}

async fn create_test_app(path: &str) -> Router {
    // Clean up Redis first
    cleanup_redis().await;

    // Redis pool setup
    let redis_cfg = RedisConfig::from_url("redis://127.0.0.1/");
    let pool = redis_cfg
        .create_pool(None)
        .expect("Failed to create Redis pool");

    // Setup API keys in Redis
    {
        let mut conn = pool.get().await.expect("Failed to get Redis connection");

        // Setup first API key
        let key = format!("barnacle:api_keys:{}", VALID_KEY);
        let config_key = format!("barnacle:api_keys:config:{}", VALID_KEY);

        let _: () = deadpool_redis::redis::cmd("SET")
            .arg(&key)
            .arg(1)
            .query_async(&mut conn)
            .await
            .expect("Failed to set API key");

        let config = barnacle_rs::BarnacleConfig {
            max_requests: RATE_LIMIT_VALID,
            window: Duration::from_secs(WINDOW_SECONDS),
            ..Default::default()
        };
        let config_json = serde_json::to_string(&config).unwrap();
        let _: () = deadpool_redis::redis::cmd("SET")
            .arg(&config_key)
            .arg(&config_json)
            .query_async(&mut conn)
            .await
            .expect("Failed to set config");

        // Setup second API key
        let key2 = format!("barnacle:api_keys:{}", VALID_KEY_2);
        let config_key2 = format!("barnacle:api_keys:config:{}", VALID_KEY_2);

        let _: () = deadpool_redis::redis::cmd("SET")
            .arg(&key2)
            .arg(1)
            .query_async(&mut conn)
            .await
            .expect("Failed to set second API key");

        let _: () = deadpool_redis::redis::cmd("SET")
            .arg(&config_key2)
            .arg(&config_json)
            .query_async(&mut conn)
            .await
            .expect("Failed to set second config");
    }

    let rate_limit_store = RedisBarnacleStore::new(pool);
    let api_key_validator = |api_key: String, _api_key_config: ApiKeyConfig, _parts: Arc<Parts>, _state: ()| async move {
        if api_key.is_empty() {
            Err(BarnacleError::ApiKeyMissing)
        } else if api_key != VALID_KEY && api_key != VALID_KEY_2 {
            Err(BarnacleError::invalid_api_key(api_key))
        } else {
            Ok(())
        }
    };
    let middleware: BarnacleLayer<(), RedisBarnacleStore, (), BarnacleError, _> = BarnacleLayer::builder()
        .with_store(rate_limit_store)
        .with_config(barnacle_rs::BarnacleConfig {
            max_requests: RATE_LIMIT_VALID,
            window: Duration::from_secs(WINDOW_SECONDS),
            ..Default::default()
        })
        .with_api_key_validator(api_key_validator)
        .with_state(())
        .build()
        .unwrap();

    // Test endpoint
    Router::new()
        .route(path, get(test_handler))
        .layer(middleware)
}

async fn test_handler() -> Json<serde_json::Value> {
    Json(json!({
        "message": "API key test successful",
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

async fn start_test_server(path: &str) -> String {
    let app = create_test_app(path).await;

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
    tokio::time::sleep(Duration::from_millis(100)).await;
    base_url
}

async fn make_request(url: &str, api_key: Option<&str>) -> (StatusCode, String) {
    let client = reqwest::Client::new();
    let mut request = client.get(url);

    if let Some(key) = api_key {
        request = request.header("x-api-key", key);
    }

    let response = request.send().await.expect("Failed to send request");
    let status = response.status();
    let body = response.text().await.expect("Failed to read response body");

    (status, body)
}

async fn print_redis_keys() {
    let redis_cfg = RedisConfig::from_url("redis://127.0.0.1/");
    let pool = redis_cfg.create_pool(None).expect("Failed to create Redis pool");
    let mut conn = pool.get().await.expect("Failed to get Redis connection");
    let _keys: Vec<String> = deadpool_redis::redis::cmd("KEYS")
        .arg("barnacle:*")
        .query_async(&mut conn)
        .await
        .unwrap_or_default();
}

mod api_keys {
    use super::*;

    #[tokio::test]
    async fn test_valid_api_key_works() {
        init_tracing();
        let base_url = start_test_server("/test").await;
        let url = format!("{}/test", base_url);

        let (status, body) = make_request(&url, Some(VALID_KEY)).await;

        assert_eq!(status, StatusCode::OK, "Should accept valid API Key");
        assert!(body.contains("API key test successful"));
    }

    #[tokio::test]
    async fn test_no_api_key_rejected() {
        init_tracing();
        let base_url = start_test_server("/test").await;
        let url = format!("{}/test", base_url);

        let (status, _body) = make_request(&url, None).await;

        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "Should reject when API Key is missing"
        );
    }

    #[tokio::test]
    async fn test_rate_limit_exceeded() {
        init_tracing();
        print_redis_keys().await;
        cleanup_redis().await;
        print_redis_keys().await;

        let unique_path = format!("/test_{}", Uuid::new_v4());
        let base_url = start_test_server(&unique_path).await;
        let url = format!("{}{}", base_url, unique_path);

        let client = reqwest::Client::new();
        let _resp1 = client.get(&url).header("x-api-key", VALID_KEY).send().await.unwrap();

        let _resp2 = client.get(&url).header("x-api-key", VALID_KEY).send().await.unwrap();


        for i in 1..=RATE_LIMIT_VALID {
            let (status, _body) = make_request(&url, Some(VALID_KEY_2)).await;
            assert_eq!(
                status,
                StatusCode::OK,
                "Request {} should be accepted (within rate limit)",
                i
            );
        }

        let (status, _body) = make_request(&url, Some(VALID_KEY_2)).await;
        assert_eq!(
            status,
            StatusCode::TOO_MANY_REQUESTS,
            "Should reject after exceeding rate limit"
        );
    }

    #[tokio::test]
    async fn test_rate_limit_headers() {
        init_tracing();
        let base_url = start_test_server("/test").await;
        let url = format!("{}/test", base_url);

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("x-api-key", VALID_KEY)
            .send()
            .await
            .expect("Failed to send request");

        let headers = response.headers();
        assert!(headers.contains_key("X-RateLimit-Limit"));
        assert!(headers.contains_key("X-RateLimit-Remaining"));
    }

    #[tokio::test]
    async fn test_redis_connection_failure() {
        init_tracing();
        // Invalid redis url
        let redis_cfg = RedisConfig::from_url("redis://invalid-host:6379/");
        let pool = redis_cfg
            .create_pool(None)
            .expect("Failed to create Redis pool");

        let rate_limit_store = RedisBarnacleStore::new(pool);
        let api_key_validator = |api_key: String, _api_key_config: ApiKeyConfig, _parts: Arc<Parts>, _state: ()| async move {
            if api_key.is_empty() {
                Err(BarnacleError::ApiKeyMissing)
            } else if api_key != VALID_KEY && api_key != VALID_KEY_2 {
                Err(BarnacleError::invalid_api_key(api_key))
            } else {
                Ok(())
            }
        };
        let middleware: BarnacleLayer<(), RedisBarnacleStore, (), BarnacleError, _> = BarnacleLayer::builder()
            .with_store(rate_limit_store)
            .with_config(barnacle_rs::BarnacleConfig {
                max_requests: RATE_LIMIT_VALID,
                window: Duration::from_secs(WINDOW_SECONDS),
                ..Default::default()
            })
            .with_api_key_validator(api_key_validator)
            .with_state(())
            .build()
            .unwrap();

        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://{}", addr);

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        tokio::time::sleep(Duration::from_millis(100)).await;

        // Should fail
        let (status, _body) = make_request(&format!("{}/test", base_url), Some(VALID_KEY)).await;
        assert!(status.is_client_error() || status.is_server_error());
    }
}
