use barnacle_rs::{BarnacleConfig, BarnacleKey, BarnacleContext, ResetOnSuccess, BarnacleResult, BarnacleError, BarnacleStore};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// Mock store for in-memory rate limiting
#[derive(Clone, Default)]
struct MockStore {
    // (key, path, method) -> (count, expiry)
    counters: Arc<Mutex<HashMap<(BarnacleKey, String, String), u32>>>,
}

#[async_trait::async_trait]
impl BarnacleStore for MockStore {
    async fn increment(&self, context: &BarnacleContext, config: &BarnacleConfig) -> Result<BarnacleResult, BarnacleError> {
        let mut counters = self.counters.lock().unwrap();
        let k = (context.key.clone(), context.path.clone(), context.method.clone());
        let count = counters.entry(k).or_insert(0);
        if *count >= config.max_requests {
            return Err(BarnacleError::rate_limit_exceeded(0, config.window.as_secs(), config.max_requests));
        }
        *count += 1;
        Ok(BarnacleResult { allowed: true, remaining: config.max_requests - *count, retry_after: None })
    }
    async fn reset(&self, context: &BarnacleContext) -> Result<(), BarnacleError> {
        let mut counters = self.counters.lock().unwrap();
        let k = (context.key.clone(), context.path.clone(), context.method.clone());
        counters.remove(&k);
        Ok(())
    }
}

fn config() -> BarnacleConfig {
    BarnacleConfig { max_requests: 2, window: Duration::from_secs(60), reset_on_success: ResetOnSuccess::Not }
}

#[cfg(test)]
mod adv_unit_tests {
    use super::*;

    #[tokio::test]
    async fn test_api_key_and_ip_isolation() {
        // Different API keys should not interfere
        let store = MockStore::default();
        let c = config();
        let ctx1 = BarnacleContext { key: BarnacleKey::ApiKey("key1".into()), path: "/a".into(), method: "GET".into() };
        let ctx2 = BarnacleContext { key: BarnacleKey::ApiKey("key2".into()), path: "/a".into(), method: "GET".into() };
        let ctx_ip = BarnacleContext { key: BarnacleKey::Ip("1.2.3.4".into()), path: "/a".into(), method: "GET".into() };
        // Each key can make 2 requests
        for _ in 0..2 { assert!(store.increment(&ctx1, &c).await.is_ok()); }
        assert!(store.increment(&ctx1, &c).await.is_err());
        for _ in 0..2 { assert!(store.increment(&ctx2, &c).await.is_ok()); }
        assert!(store.increment(&ctx2, &c).await.is_err());
        for _ in 0..2 { assert!(store.increment(&ctx_ip, &c).await.is_ok()); }
        assert!(store.increment(&ctx_ip, &c).await.is_err());
    }

    #[tokio::test]
    async fn test_payload_key_extraction_edge_cases() {
        // Simulate payload-based keys: missing, malformed, duplicate
        let store = MockStore::default();
        let c = config();
        // Missing key (should fallback to IP or error)
        let ctx_missing = BarnacleContext { key: BarnacleKey::Custom("".into()), path: "/b".into(), method: "POST".into() };
        assert!(store.increment(&ctx_missing, &c).await.is_ok());
        // Malformed key (simulate as Custom with garbage)
        let ctx_malformed = BarnacleContext { key: BarnacleKey::Custom("{notjson}".into()), path: "/b".into(), method: "POST".into() };
        assert!(store.increment(&ctx_malformed, &c).await.is_ok());
        // Duplicate keys (should be treated as separate)
        let ctx_dup1 = BarnacleContext { key: BarnacleKey::Custom("dup".into()), path: "/b".into(), method: "POST".into() };
        let ctx_dup2 = BarnacleContext { key: BarnacleKey::Custom("dup".into()), path: "/b".into(), method: "POST".into() };
        for _ in 0..2 { assert!(store.increment(&ctx_dup1, &c).await.is_ok()); }
        assert!(store.increment(&ctx_dup2, &c).await.is_err());
    }

    #[tokio::test]
    async fn test_fallback_logic_and_empty_keys() {
        // If no API key and no payload key, fallback to IP
        let store = MockStore::default();
        let c = config();
        let ctx_fallback = BarnacleContext { key: BarnacleKey::Ip("127.0.0.1".into()), path: "/c".into(), method: "GET".into() };
        for _ in 0..2 { assert!(store.increment(&ctx_fallback, &c).await.is_ok()); }
        assert!(store.increment(&ctx_fallback, &c).await.is_err());
        // Empty API key (should be treated as unique key)
        let ctx_empty = BarnacleContext { key: BarnacleKey::ApiKey("".into()), path: "/c".into(), method: "GET".into() };
        for _ in 0..2 { assert!(store.increment(&ctx_empty, &c).await.is_ok()); }
        assert!(store.increment(&ctx_empty, &c).await.is_err());
    }

    #[tokio::test]
    async fn test_path_and_method_isolation() {
        // Same key, different path or method should not interfere
        let store = MockStore::default();
        let c = config();
        let ctx1 = BarnacleContext { key: BarnacleKey::ApiKey("key".into()), path: "/d1".into(), method: "GET".into() };
        let ctx2 = BarnacleContext { key: BarnacleKey::ApiKey("key".into()), path: "/d2".into(), method: "GET".into() };
        let ctx3 = BarnacleContext { key: BarnacleKey::ApiKey("key".into()), path: "/d1".into(), method: "POST".into() };
        for _ in 0..2 { assert!(store.increment(&ctx1, &c).await.is_ok()); }
        assert!(store.increment(&ctx1, &c).await.is_err());
        for _ in 0..2 { assert!(store.increment(&ctx2, &c).await.is_ok()); }
        assert!(store.increment(&ctx2, &c).await.is_err());
        for _ in 0..2 { assert!(store.increment(&ctx3, &c).await.is_ok()); }
        assert!(store.increment(&ctx3, &c).await.is_err());
    }

    #[tokio::test]
    async fn test_rapid_switching_between_keys() {
        // Rapidly alternate between keys to try to bypass limits
        let store = super::MockStore::default();
        let c = super::config();
        let keys = ["a", "b", "c", "d"];
        let ctxs: Vec<_> = keys.iter().map(|k| BarnacleContext { key: BarnacleKey::ApiKey((*k).into()), path: "/e".into(), method: "GET".into() }).collect();
        for _ in 0..2 {
            for ctx in &ctxs {
                assert!(store.increment(ctx, &c).await.is_ok());
            }
        }
        for ctx in &ctxs {
            assert!(store.increment(ctx, &c).await.is_err());
        }
    }

    #[tokio::test]
    async fn test_attempted_bypass_with_header_spoofing() {
        // Simulate header spoofing: same IP, different API key, or vice versa
        let store = MockStore::default();
        let c = config();
        let ctx_ip = BarnacleContext { key: BarnacleKey::Ip("1.2.3.4".into()), path: "/f".into(), method: "GET".into() };
        let ctx_api = BarnacleContext { key: BarnacleKey::ApiKey("spoofed".into()), path: "/f".into(), method: "GET".into() };
        for _ in 0..2 { assert!(store.increment(&ctx_ip, &c).await.is_ok()); }
        assert!(store.increment(&ctx_ip, &c).await.is_err());
        for _ in 0..2 { assert!(store.increment(&ctx_api, &c).await.is_ok()); }
        assert!(store.increment(&ctx_api, &c).await.is_err());
    }

    #[tokio::test]
    async fn test_reset_on_success_and_error() {
        // Test that reset works and only for the right context
        let store = MockStore::default();
        let c = config();
        let ctx = BarnacleContext { key: BarnacleKey::ApiKey("resetme".into()), path: "/g".into(), method: "GET".into() };
        for _ in 0..2 { assert!(store.increment(&ctx, &c).await.is_ok()); }
        assert!(store.increment(&ctx, &c).await.is_err());
        // Reset
        assert!(store.reset(&ctx).await.is_ok());
        // Should be allowed again
        for _ in 0..2 { assert!(store.increment(&ctx, &c).await.is_ok()); }
        assert!(store.increment(&ctx, &c).await.is_err());
    }

    #[tokio::test]
    async fn test_api_key_with_whitespace_and_unicode() {
        // API keys with whitespace or unicode should be treated as unique
        let store = super::MockStore::default();
        let c = super::config();
        let ctx_ws = BarnacleContext { key: BarnacleKey::ApiKey("key with space".into()), path: "/h".into(), method: "GET".into() };
        let ctx_unicode = BarnacleContext { key: BarnacleKey::ApiKey("ключ".into()), path: "/h".into(), method: "GET".into() };
        for _ in 0..2 { assert!(store.increment(&ctx_ws, &c).await.is_ok()); }
        assert!(store.increment(&ctx_ws, &c).await.is_err());
        for _ in 0..2 { assert!(store.increment(&ctx_unicode, &c).await.is_ok()); }
        assert!(store.increment(&ctx_unicode, &c).await.is_err());
    }

    #[tokio::test]
    async fn test_api_key_case_sensitivity() {
        // API keys should be case sensitive
        let store = super::MockStore::default();
        let c = super::config();
        let ctx_lower = BarnacleContext { key: BarnacleKey::ApiKey("casekey".into()), path: "/i".into(), method: "GET".into() };
        let ctx_upper = BarnacleContext { key: BarnacleKey::ApiKey("CASEKEY".into()), path: "/i".into(), method: "GET".into() };
        for _ in 0..2 { assert!(store.increment(&ctx_lower, &c).await.is_ok()); }
        assert!(store.increment(&ctx_lower, &c).await.is_err());
        for _ in 0..2 { assert!(store.increment(&ctx_upper, &c).await.is_ok()); }
        assert!(store.increment(&ctx_upper, &c).await.is_err());
    }

    #[tokio::test]
    async fn test_path_trailing_slash() {
        // /j and /j/ should be treated as different paths
        let store = super::MockStore::default();
        let c = super::config();
        let ctx1 = BarnacleContext { key: BarnacleKey::ApiKey("key".into()), path: "/j".into(), method: "GET".into() };
        let ctx2 = BarnacleContext { key: BarnacleKey::ApiKey("key".into()), path: "/j/".into(), method: "GET".into() };
        for _ in 0..2 { assert!(store.increment(&ctx1, &c).await.is_ok()); }
        assert!(store.increment(&ctx1, &c).await.is_err());
        for _ in 0..2 { assert!(store.increment(&ctx2, &c).await.is_ok()); }
        assert!(store.increment(&ctx2, &c).await.is_err());
    }

    #[tokio::test]
    async fn test_method_spoofing_case() {
        // Method should be case sensitive (GET vs get)
        let store = super::MockStore::default();
        let c = super::config();
        let ctx_upper = BarnacleContext { key: BarnacleKey::ApiKey("key".into()), path: "/k".into(), method: "GET".into() };
        let ctx_lower = BarnacleContext { key: BarnacleKey::ApiKey("key".into()), path: "/k".into(), method: "get".into() };
        for _ in 0..2 { assert!(store.increment(&ctx_upper, &c).await.is_ok()); }
        assert!(store.increment(&ctx_upper, &c).await.is_err());
        for _ in 0..2 { assert!(store.increment(&ctx_lower, &c).await.is_ok()); }
        assert!(store.increment(&ctx_lower, &c).await.is_err());
    }

    #[tokio::test]
    async fn test_very_long_api_key_and_path() {
        // Very long API key and path should not break the store
        let store = super::MockStore::default();
        let c = super::config();
        let long_key = "k".repeat(1024);
        let long_path = format!("/{}", "p".repeat(1024));
        let ctx = BarnacleContext { key: BarnacleKey::ApiKey(long_key), path: long_path, method: "GET".into() };
        for _ in 0..2 { assert!(store.increment(&ctx, &c).await.is_ok()); }
        assert!(store.increment(&ctx, &c).await.is_err());
    }

    #[tokio::test]
    async fn test_multiple_resets_in_a_row() {
        // Multiple resets should not panic or break
        let store = super::MockStore::default();
        let c = super::config();
        let ctx = BarnacleContext { key: BarnacleKey::ApiKey("resetmany".into()), path: "/l".into(), method: "GET".into() };
        for _ in 0..2 { assert!(store.increment(&ctx, &c).await.is_ok()); }
        assert!(store.increment(&ctx, &c).await.is_err());
        for _ in 0..3 { assert!(store.reset(&ctx).await.is_ok()); }
        for _ in 0..2 { assert!(store.increment(&ctx, &c).await.is_ok()); }
        assert!(store.increment(&ctx, &c).await.is_err());
    }

    #[tokio::test]
    async fn test_simultaneous_requests_simulated() {
        // Simulate concurrent requests (not truly parallel, but interleaved)
        let store = super::MockStore::default();
        let c = super::config();
        let ctx = BarnacleContext { key: BarnacleKey::ApiKey("concurrent".into()), path: "/m".into(), method: "GET".into() };
        let futs: Vec<_> = (0..2).map(|_| store.increment(&ctx, &c)).collect();
        let results = futures::future::join_all(futs).await;
        assert!(results.iter().all(|r| r.is_ok()));
        assert!(store.increment(&ctx, &c).await.is_err());
    }

    #[tokio::test]
    async fn test_overlapping_keys() {
        // ApiKey("foo") and Custom("foo") should be treated as different
        let store = super::MockStore::default();
        let c = super::config();
        let ctx_api = BarnacleContext { key: BarnacleKey::ApiKey("foo".into()), path: "/n".into(), method: "GET".into() };
        let ctx_custom = BarnacleContext { key: BarnacleKey::Custom("foo".into()), path: "/n".into(), method: "GET".into() };
        for _ in 0..2 { assert!(store.increment(&ctx_api, &c).await.is_ok()); }
        assert!(store.increment(&ctx_api, &c).await.is_err());
        for _ in 0..2 { assert!(store.increment(&ctx_custom, &c).await.is_ok()); }
        assert!(store.increment(&ctx_custom, &c).await.is_err());
    }

    #[tokio::test]
    async fn test_reset_nonexistent_context() {
        // Resetting a non-existent context should not panic or error
        let store = super::MockStore::default();
        let ctx = BarnacleContext { key: BarnacleKey::ApiKey("nope".into()), path: "/o".into(), method: "GET".into() };
        assert!(store.reset(&ctx).await.is_ok());
    }

    #[tokio::test]
    async fn test_increment_after_reset_wrong_context() {
        // Resetting a different context should not affect others
        let store = super::MockStore::default();
        let c = super::config();
        let ctx1 = BarnacleContext { key: BarnacleKey::ApiKey("p1".into()), path: "/p".into(), method: "GET".into() };
        let ctx2 = BarnacleContext { key: BarnacleKey::ApiKey("p2".into()), path: "/p".into(), method: "GET".into() };
        for _ in 0..2 { assert!(store.increment(&ctx1, &c).await.is_ok()); }
        assert!(store.increment(&ctx1, &c).await.is_err());
        assert!(store.reset(&ctx2).await.is_ok());
        assert!(store.increment(&ctx1, &c).await.is_err());
    }
} 