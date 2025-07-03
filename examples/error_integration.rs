use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use barnacle_rs::{BarnacleError, FromBarnacleError};
use serde_json::json;
use thiserror::Error;
use std::collections::HashMap;

/// Example of an application-specific error enum that can convert from BarnacleError
#[derive(Error, Debug)]
pub enum AppError {
    #[error("Authentication failed: {0}")]
    Authentication(String),
    
    #[error("Rate limiting error: {0}")]
    RateLimit(#[from] BarnacleError),
    
    #[error("Database error: {message}")]
    Database { message: String },
    
    #[error("Validation error: {0}")]
    Validation(String),
    
    #[error("Internal server error: {0}")]
    Internal(String),
}

impl AppError {
    pub fn status_code(&self) -> StatusCode {
        match self {
            AppError::Authentication(_) => StatusCode::UNAUTHORIZED,
            AppError::RateLimit(barnacle_error) => barnacle_error.status_code(),
            AppError::Database { .. } => StatusCode::SERVICE_UNAVAILABLE,
            AppError::Validation(_) => StatusCode::BAD_REQUEST,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn error_code(&self) -> &'static str {
        match self {
            AppError::Authentication(_) => "AUTH_ERROR",
            AppError::RateLimit(barnacle_error) => barnacle_error.error_code(),
            AppError::Database { .. } => "DATABASE_ERROR",
            AppError::Validation(_) => "VALIDATION_ERROR",
            AppError::Internal(_) => "INTERNAL_ERROR",
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        
        // For rate limit errors, delegate to BarnacleError's response
        if let AppError::RateLimit(barnacle_error) = self {
            return barnacle_error.into_response();
        }
        
        let body = Json(json!({
            "error": {
                "code": self.error_code(),
                "message": self.to_string(),
                "type": "application_error"
            }
        }));

        (status, body).into_response()
    }
}

// Implement the FromBarnacleError trait to enable easy conversion
impl FromBarnacleError<AppError> for AppError {
    fn from_barnacle_error(error: BarnacleError) -> AppError {
        AppError::RateLimit(error)
    }
}

// Alternative: Use the macro for simpler implementation
// barnacle_rs::impl_from_barnacle_error!(AppError, RateLimit);

/// Example handler that might encounter BarnacleError
async fn protected_handler() -> Result<Json<serde_json::Value>, AppError> {
    // Simulate some operation that might fail with a BarnacleError
    let result = simulate_barnacle_operation().await;
    
    match result {
        Ok(data) => Ok(Json(json!({
            "message": "Success",
            "data": data
        }))),
        Err(barnacle_error) => {
            // Convert BarnacleError to AppError
            Err(AppError::from_barnacle_error(barnacle_error))
        }
    }
}

/// Example handler that demonstrates manual error conversion
async fn manual_conversion_handler() -> Result<Json<serde_json::Value>, AppError> {
    let result = simulate_barnacle_operation().await;
    
    match result {
        Ok(data) => Ok(Json(json!({
            "message": "Success",
            "data": data
        }))),
        Err(barnacle_error) => {
            // Manual conversion with additional context
            match barnacle_error {
                BarnacleError::RateLimitExceeded { remaining, retry_after, limit } => {
                    // You could transform this into your own error type
                    Err(AppError::RateLimit(BarnacleError::rate_limit_exceeded(
                        remaining, retry_after, limit
                    ).with_context("User exceeded API rate limit")))
                },
                BarnacleError::ApiKeyMissing => {
                    Err(AppError::Authentication("API key is required for this endpoint".to_string()))
                },
                BarnacleError::InvalidApiKey { key_hint } => {
                    Err(AppError::Authentication(format!("Invalid API key: {}", key_hint)))
                },
                other => {
                    // For other errors, wrap as internal error
                    Err(AppError::Internal(format!("Service error: {}", other)))
                }
            }
        }
    }
}

/// Example handler showing how to add context to BarnacleErrors
async fn context_handler() -> Result<Json<serde_json::Value>, AppError> {
    let result = simulate_barnacle_operation().await
        .map_err(|err| err.with_context("Failed during user data processing"))?;
    
    Ok(Json(json!({
        "message": "Success",
        "data": result
    })))
}

/// Simulate a function that returns a BarnacleError
async fn simulate_barnacle_operation() -> Result<HashMap<String, String>, BarnacleError> {
    // Simulate different types of errors
    let error_type = std::env::var("SIMULATE_ERROR").unwrap_or_default();
    
    match error_type.as_str() {
        "rate_limit" => Err(BarnacleError::rate_limit_exceeded(0, 60, 100)),
        "api_key_missing" => Err(BarnacleError::ApiKeyMissing),
        "invalid_key" => Err(BarnacleError::invalid_api_key("test_key_123")),
        "store_error" => Err(BarnacleError::store_error("Redis connection failed")),
        "custom" => Err(BarnacleError::custom("Custom application error", Some(StatusCode::CONFLICT))),
        _ => {
            let mut data = HashMap::new();
            data.insert("key".to_string(), "value".to_string());
            Ok(data)
        }
    }
}

/// Example middleware that converts BarnacleError to AppError
async fn error_conversion_middleware<B>(
    req: axum::extract::Request<B>,
    next: axum::middleware::Next<B>,
) -> Result<Response, AppError> {
    let response = next.run(req).await;
    
    // If the response is an error that contains BarnacleError information,
    // you could inspect and transform it here
    Ok(response)
}

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::init();

    // Build the application with error handling
    let app = Router::new()
        .route("/protected", get(protected_handler))
        .route("/manual", get(manual_conversion_handler))
        .route("/context", get(context_handler))
        .layer(axum::middleware::from_fn(error_conversion_middleware));

    println!("Starting server on http://localhost:3000");
    println!("Try the following endpoints:");
    println!("  GET /protected - Basic error conversion");
    println!("  GET /manual - Manual error handling");
    println!("  GET /context - Error with context");
    println!();
    println!("Set SIMULATE_ERROR environment variable to test different errors:");
    println!("  rate_limit, api_key_missing, invalid_key, store_error, custom");

    // Start the server
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_error_conversion() {
        let app = Router::new().route("/test", get(protected_handler));

        // Test with successful case
        std::env::set_var("SIMULATE_ERROR", "");
        let response = app
            .clone()
            .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Test with rate limit error
        std::env::set_var("SIMULATE_ERROR", "rate_limit");
        let response = app
            .clone()
            .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

        // Test with API key missing error
        std::env::set_var("SIMULATE_ERROR", "api_key_missing");
        let response = app
            .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn test_barnacle_error_properties() {
        let error = BarnacleError::rate_limit_exceeded(5, 30, 100);
        
        assert_eq!(error.status_code(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(error.error_code(), "RATE_LIMIT_EXCEEDED");
        assert_eq!(error.error_type(), "rate_limit");
        assert!(error.is_retryable());
        assert_eq!(error.retry_after(), Some(30));
    }

    #[test]
    fn test_error_context() {
        let error = BarnacleError::store_error("Connection failed")
            .with_context("Database operation");
        
        assert!(error.to_string().contains("Database operation"));
        assert!(error.to_string().contains("Connection failed"));
    }
}