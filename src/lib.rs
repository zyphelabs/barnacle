//! Custom rate limiting library with Redis and Axum support

pub mod backoff;
pub mod middleware;
pub mod redis_store;
pub mod types;

use std::sync::Arc;
use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use types::{BarnacleKey, BarnacleResult, BarnacleConfig};

/// Trait to abstract the rate limiter storage backend (e.g., Redis)
#[async_trait]
pub trait BarnacleStore: Send + Sync {
    /// Increments the counter for the key and returns the current number of requests and remaining time until reset.
    async fn increment(&self, key: &BarnacleKey, config: &BarnacleConfig) -> BarnacleResult;
    /// Resets the counter for the key (e.g., after successful login).
    async fn reset(&self, key: &BarnacleKey) -> anyhow::Result<()>;
}

/// In-memory store for testing purposes
pub struct MemoryBarnacleStore {
    counters: Arc<Mutex<HashMap<String, (u32, std::time::Instant)>>>,
}

impl MemoryBarnacleStore {
    pub fn new() -> Self {
        Self {
            counters: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn get_key_string(&self, key: &BarnacleKey) -> String {
        match key {
            BarnacleKey::Email(email) => format!("email:{}", email),
            BarnacleKey::ApiKey(api_key) => format!("api_key:{}", api_key),
            BarnacleKey::Ip(ip) => format!("ip:{}", ip),
        }
    }
}

#[async_trait]
impl BarnacleStore for MemoryBarnacleStore {
    async fn increment(&self, key: &BarnacleKey, config: &BarnacleConfig) -> BarnacleResult {
        let key_string = self.get_key_string(key);
        let now = std::time::Instant::now();
        
        let mut counters = self.counters.lock().unwrap();
        
        if let Some((count, window_start)) = counters.get(&key_string) {
            let elapsed = now.duration_since(*window_start);
            
            if elapsed >= config.window {
                // Window expired, reset counter
                counters.insert(key_string, (1, now));
                return BarnacleResult {
                    allowed: true,
                    remaining: config.max_requests - 1,
                    retry_after: None,
                };
            } else {
                // Within window
                if *count >= config.max_requests {
                    // Rate limit exceeded
                    let retry_after = config.window - elapsed;
                    return BarnacleResult {
                        allowed: false,
                        remaining: 0,
                        retry_after: Some(retry_after),
                    };
                } else {
                    // Increment counter
                    let new_count = count + 1;
                    let window_start_copy = *window_start;
                    counters.insert(key_string, (new_count, window_start_copy));
                    return BarnacleResult {
                        allowed: true,
                        remaining: config.max_requests - new_count,
                        retry_after: None,
                    };
                }
            }
        } else {
            // First request for this key
            counters.insert(key_string, (1, now));
            return BarnacleResult {
                allowed: true,
                remaining: config.max_requests - 1,
                retry_after: None,
            };
        }
    }

    async fn reset(&self, key: &BarnacleKey) -> anyhow::Result<()> {
        let key_string = self.get_key_string(key);
        let mut counters = self.counters.lock().unwrap();
        counters.remove(&key_string);
        Ok(())
    }
}

/// Utility function to create the rate limiting middleware
pub fn barnacle_layer<S: BarnacleStore + 'static>(
    store: Arc<S>,
    config: BarnacleConfig,
) -> middleware::BarnacleLayer<S> {
    // Default extractor: always None, so fallback to IP
    let extractor: std::sync::Arc<dyn Fn(&axum::http::Request<axum::body::Body>) -> Option<BarnacleKey> + Send + Sync> =
    std::sync::Arc::new(|_req| None);
    middleware::BarnacleLayer::new(store, config, extractor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_basic_rate_limiting() {
        let store = Arc::new(MemoryBarnacleStore::new());
        let config = BarnacleConfig {
            max_requests: 3,
            window: Duration::from_secs(60),
            backoff: None,
            reset_on_success: false,
        };

        let key = BarnacleKey::Ip("127.0.0.1".to_string());

        // First 3 requests should be allowed
        for i in 0..3 {
            let result = store.increment(&key, &config).await;
            assert!(result.allowed, "Request {} should be allowed", i + 1);
            assert_eq!(result.remaining, 2 - i);
        }

        // 4th request should be blocked
        let result = store.increment(&key, &config).await;
        assert!(!result.allowed, "4th request should be blocked");
        assert_eq!(result.remaining, 0);
        assert!(result.retry_after.is_some());
    }

    #[tokio::test]
    async fn test_rate_limit_reset() {
        let store = Arc::new(MemoryBarnacleStore::new());
        let config = BarnacleConfig {
            max_requests: 2,
            window: Duration::from_secs(60),
            backoff: None,
            reset_on_success: false,
        };

        let key = BarnacleKey::Email("test@example.com".to_string());

        // Make 2 requests
        for _ in 0..2 {
            let result = store.increment(&key, &config).await;
            assert!(result.allowed);
        }

        // 3rd request should be blocked
        let result = store.increment(&key, &config).await;
        assert!(!result.allowed);

        // Reset the rate limit
        store.reset(&key).await.unwrap();

        // Should be able to make requests again
        let result = store.increment(&key, &config).await;
        assert!(result.allowed);
        assert_eq!(result.remaining, 1);
    }

    #[tokio::test]
    async fn test_different_key_types() {
        let store = Arc::new(MemoryBarnacleStore::new());
        let config = BarnacleConfig {
            max_requests: 1,
            window: Duration::from_secs(60),
            backoff: None,
            reset_on_success: false,
        };

        let ip_key = BarnacleKey::Ip("127.0.0.1".to_string());
        let email_key = BarnacleKey::Email("test@example.com".to_string());
        let api_key = BarnacleKey::ApiKey("api_key_123".to_string());

        // Each key type should be tracked separately
        let result1 = store.increment(&ip_key, &config).await;
        let result2 = store.increment(&email_key, &config).await;
        let result3 = store.increment(&api_key, &config).await;

        assert!(result1.allowed);
        assert!(result2.allowed);
        assert!(result3.allowed);

        // Second request for each should be blocked
        let result1 = store.increment(&ip_key, &config).await;
        let result2 = store.increment(&email_key, &config).await;
        let result3 = store.increment(&api_key, &config).await;

        assert!(!result1.allowed);
        assert!(!result2.allowed);
        assert!(!result3.allowed);
    }

    #[tokio::test]
    async fn test_window_expiration() {
        let store = Arc::new(MemoryBarnacleStore::new());
        let config = BarnacleConfig {
            max_requests: 1,
            window: Duration::from_millis(100), // Very short window for testing
            backoff: None,
            reset_on_success: false,
        };

        let key = BarnacleKey::Ip("127.0.0.1".to_string());

        // First request should be allowed
        let result = store.increment(&key, &config).await;
        assert!(result.allowed);

        // Second request should be blocked
        let result = store.increment(&key, &config).await;
        assert!(!result.allowed);

        // Wait for window to expire
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Should be able to make requests again
        let result = store.increment(&key, &config).await;
        assert!(result.allowed);
    }
}
