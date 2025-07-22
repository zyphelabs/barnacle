use barnacle_rs::{BarnacleConfig, BarnacleKey, ResetOnSuccess};
use std::time::Duration;

#[cfg(test)]
mod basic_unit_tests {
    use super::*;

    #[test]
    fn test_barnacle_config_creation() {
        let config = BarnacleConfig {
            max_requests: 10,
            window: Duration::from_secs(60),
            reset_on_success: ResetOnSuccess::Not,
        };

        assert_eq!(config.max_requests, 10);
        assert_eq!(config.window, Duration::from_secs(60));
        assert!(matches!(config.reset_on_success, ResetOnSuccess::Not));
    }

    #[test]
    fn test_barnacle_config_with_reset_on_success() {
        let config = BarnacleConfig {
            max_requests: 5,
            window: Duration::from_secs(300),
            reset_on_success: ResetOnSuccess::Yes(None),
        };

        assert_eq!(config.max_requests, 5);
        assert_eq!(config.window, Duration::from_secs(300));
        assert!(matches!(config.reset_on_success, ResetOnSuccess::Yes(None)));
    }

    #[test]
    fn test_barnacle_key_variants() {
        let email_key = BarnacleKey::Email("test@example.com".to_string());
        let ip_key = BarnacleKey::Ip("192.168.1.1".to_string());
        let api_key = BarnacleKey::ApiKey("api_key_123".to_string());

        // Test that keys are created correctly
        match email_key {
            BarnacleKey::Email(email) => assert_eq!(email, "test@example.com"),
            _ => panic!("Expected Email key"),
        }

        match ip_key {
            BarnacleKey::Ip(ip) => assert_eq!(ip, "192.168.1.1"),
            _ => panic!("Expected IP key"),
        }

        match api_key {
            BarnacleKey::ApiKey(key) => assert_eq!(key, "api_key_123"),
            _ => panic!("Expected ApiKey key"),
        }
    }

    #[test]
    fn test_barnacle_key_display() {
        let email_key = BarnacleKey::Email("user@domain.com".to_string());
        let ip_key = BarnacleKey::Ip("10.0.0.1".to_string());
        let api_key = BarnacleKey::ApiKey("secret_key".to_string());

        // Test string representation (assuming Display is implemented)
        assert_eq!(format!("{:?}", email_key), "Email(\"user@domain.com\")");
        assert_eq!(format!("{:?}", ip_key), "Ip(\"10.0.0.1\")");
        assert_eq!(format!("{:?}", api_key), "ApiKey(\"secret_key\")");
    }

    #[test]
    fn test_reset_on_success_variants() {
        let no_reset = ResetOnSuccess::Not;
        let reset_all = ResetOnSuccess::Yes(None);
        let reset_specific = ResetOnSuccess::Yes(Some(vec![200, 201, 204]));

        assert!(matches!(no_reset, ResetOnSuccess::Not));
        assert!(matches!(reset_all, ResetOnSuccess::Yes(None)));

        if let ResetOnSuccess::Yes(Some(codes)) = reset_specific {
            assert_eq!(codes, vec![200, 201, 204]);
        } else {
            panic!("Expected ResetOnSuccess::Yes with specific codes");
        }
    }

    #[test]
    fn test_duration_configurations() {
        // Test common duration configurations
        let short_window = Duration::from_secs(60); // 1 minute
        let medium_window = Duration::from_secs(300); // 5 minutes
        let long_window = Duration::from_secs(3600); // 1 hour

        assert_eq!(short_window.as_secs(), 60);
        assert_eq!(medium_window.as_secs(), 300);
        assert_eq!(long_window.as_secs(), 3600);

        // Test that durations work in configs
        let configs = [
            BarnacleConfig {
                max_requests: 100,
                window: short_window,
                reset_on_success: ResetOnSuccess::Not,
            },
            BarnacleConfig {
                max_requests: 10,
                window: medium_window,
                reset_on_success: ResetOnSuccess::Yes(None),
            },
            BarnacleConfig {
                max_requests: 1000,
                window: long_window,
                reset_on_success: ResetOnSuccess::Yes(Some(vec![200])),
            },
        ];

        assert_eq!(configs.len(), 3);
        assert_eq!(configs[0].max_requests, 100);
        assert_eq!(configs[1].window, Duration::from_secs(300));
        assert!(matches!(
            configs[2].reset_on_success,
            ResetOnSuccess::Yes(Some(_))
        ));
    }
}

#[cfg(test)]
mod barnacle_layer_unit_tests {
    use axum::{body::Body, http::{request::Parts, Request}};
    use barnacle_rs::{ApiKeyConfig, BarnacleError};
    use reqwest::Method;
    use std::sync::Arc;


    #[test]
    fn test_stateless_api_key_validator_success() {
        #[derive(Clone)]
        struct State { allowed: String }
        let state = State { allowed: "test-key".to_string() };
        let api_key_validator = |api_key: String, _api_key_config: ApiKeyConfig, _parts: Arc<Parts>, state: State| async move {
            let key = api_key.to_string();
            let allowed = state.allowed == key;
            if allowed {
                Ok(())
            } else {
                Err(BarnacleError::invalid_api_key(api_key))
            }
        };
        let req = Request::builder().uri("/test").method(Method::GET).body(Body::empty()).unwrap();
        let (parts, _) = req.into_parts();
        let parts = Arc::new(parts);
        // Just check that the validator works as expected
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(api_key_validator("test-key".to_string(), ApiKeyConfig::default(), parts.clone(), state.clone()));
        assert!(result.is_ok());
        let result = rt.block_on(api_key_validator("invalid".to_string(), ApiKeyConfig::default(), parts, state));
        assert!(result.is_err());
    }

    #[test]
    fn test_stateful_api_key_validator_success() {
        #[derive(Clone)]
        struct State { allowed: String }
        let state = State { allowed: "foo".to_string() };
        let validator = |api_key: String, _api_key_config: ApiKeyConfig, _parts: Arc<Parts>, state: State| async move {
            let key = api_key.to_string();
            let allowed = state.allowed == key;
            if allowed {
                Ok(())
            } else {
                Err(BarnacleError::invalid_api_key(key))
            }
        };
        let req = Request::builder().uri("/test").method(Method::GET).body(Body::empty()).unwrap();
        let (parts, _) = req.into_parts();
        let parts = Arc::new(parts);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(validator("foo".to_string(), ApiKeyConfig::default(), parts.clone(), state.clone()));
        assert!(result.is_ok());
        let result = rt.block_on(validator("bar".to_string(), ApiKeyConfig::default(), parts, state));
        assert!(result.is_err());
    }
}
