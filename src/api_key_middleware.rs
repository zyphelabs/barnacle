use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderMap, Response, StatusCode};
use std::sync::Arc;
use std::task::{Context, Poll};
use tower::{Layer, Service};
use tracing;

use crate::api_key_store::ApiKeyStore;
use crate::types::{ApiKeyMiddlewareConfig, BarnacleKey};
use crate::{BarnacleConfig, BarnacleStore};

/// Layer for API key validation and rate limiting
pub struct ApiKeyLayer<A, S> {
    api_key_store: Arc<A>,
    rate_limit_store: Arc<S>,
    config: ApiKeyMiddlewareConfig,
}

impl<A, S> Clone for ApiKeyLayer<A, S> {
    fn clone(&self) -> Self {
        Self {
            api_key_store: self.api_key_store.clone(),
            rate_limit_store: self.rate_limit_store.clone(),
            config: self.config.clone(),
        }
    }
}

impl<A, S> ApiKeyLayer<A, S>
where
    A: ApiKeyStore + 'static,
    S: BarnacleStore + 'static,
{
    pub fn new(
        api_key_store: Arc<A>,
        rate_limit_store: Arc<S>,
        config: ApiKeyMiddlewareConfig,
    ) -> Self {
        Self {
            api_key_store,
            rate_limit_store,
            config,
        }
    }
}

impl<Inner, A, S> Layer<Inner> for ApiKeyLayer<A, S>
where
    A: ApiKeyStore + 'static,
    S: BarnacleStore + 'static,
{
    type Service = ApiKeyMiddleware<Inner, A, S>;

    fn layer(&self, inner: Inner) -> Self::Service {
        ApiKeyMiddleware {
            inner,
            api_key_store: self.api_key_store.clone(),
            rate_limit_store: self.rate_limit_store.clone(),
            config: self.config.clone(),
        }
    }
}

/// The actual API key validation middleware
pub struct ApiKeyMiddleware<Inner, A, S> {
    inner: Inner,
    api_key_store: Arc<A>,
    rate_limit_store: Arc<S>,
    config: ApiKeyMiddlewareConfig,
}

impl<Inner, A, S> Clone for ApiKeyMiddleware<Inner, A, S>
where
    Inner: Clone,
    A: ApiKeyStore + 'static,
    S: BarnacleStore + 'static,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            api_key_store: self.api_key_store.clone(),
            rate_limit_store: self.rate_limit_store.clone(),
            config: self.config.clone(),
        }
    }
}

impl<Inner, B, A, S> Service<Request<B>> for ApiKeyMiddleware<Inner, A, S>
where
    Inner: Service<Request<B>, Response = Response<Body>> + Clone + Send + 'static,
    Inner::Future: Send + 'static,
    B: Send + 'static,
    A: ApiKeyStore + 'static,
    S: BarnacleStore + 'static,
{
    type Response = Inner::Response;
    type Error = Inner::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        let mut inner = self.inner.clone();
        let api_key_store = self.api_key_store.clone();
        let rate_limit_store = self.rate_limit_store.clone();
        let config = self.config.clone();

        Box::pin(async move {
            let headers = req.headers();

            // Extract API key from headers
            let api_key = extract_api_key(headers, &config.header_name);

            // Handle missing API key
            if api_key.is_none() && config.require_api_key {
                tracing::warn!("API key missing in header: {}", config.header_name);
                return Ok(create_unauthorized_response("API key required"));
            }

            // If we have an API key, validate it
            if let Some(api_key) = api_key {
                let validation_result = api_key_store.validate_key(&api_key).await;

                if !validation_result.valid {
                    tracing::warn!("Invalid API key: {}", api_key);
                    return Ok(create_unauthorized_response("Invalid API key"));
                }

                // Get rate limit configuration for this key
                let rate_limit_config = validation_result
                    .rate_limit_config
                    .unwrap_or_else(|| config.default_rate_limit.clone());

                // Create rate limiting key
                let rate_limit_key = BarnacleKey::ApiKey(api_key.clone());

                // Check rate limit
                let rate_limit_result = rate_limit_store
                    .increment(&rate_limit_key, &rate_limit_config)
                    .await;

                if !rate_limit_result.allowed {
                    tracing::warn!(
                        "Rate limit exceeded for API key: {}, remaining: {}, retry_after: {:?}",
                        api_key,
                        rate_limit_result.remaining,
                        rate_limit_result.retry_after
                    );
                    return Ok(create_rate_limit_response(
                        rate_limit_result,
                        &rate_limit_config,
                    ));
                }

                tracing::debug!(
                    "API key validation and rate limit check passed for: {}, remaining: {}",
                    api_key,
                    rate_limit_result.remaining
                );

                let mut response = inner.call(req).await?;

                let headers = response.headers_mut();
                if let Ok(remaining_header) = rate_limit_result.remaining.to_string().parse() {
                    headers.insert("X-RateLimit-Remaining", remaining_header);
                }
                if let Ok(limit_header) = rate_limit_config.max_requests.to_string().parse() {
                    headers.insert("X-RateLimit-Limit", limit_header);
                }
                if let Some(retry_after) = rate_limit_result.retry_after {
                    if let Ok(reset_header) = retry_after.as_secs().to_string().parse() {
                        headers.insert("X-RateLimit-Reset", reset_header);
                    }
                }

                // Handle rate limit reset on success if configured
                handle_rate_limit_reset(
                    &rate_limit_store,
                    &rate_limit_config,
                    &rate_limit_key,
                    response.status().as_u16(),
                )
                .await;

                Ok(response)
            } else {
                // No API key required, continue without validation
                inner.call(req).await
            }
        })
    }
}

fn extract_api_key(headers: &HeaderMap, header_name: &str) -> Option<String> {
    for (name, value) in headers.iter() {
        if name.as_str().eq_ignore_ascii_case(header_name) {
            return value.to_str().ok().map(|s| s.to_string());
        }
    }
    None
}

fn create_unauthorized_response(message: &str) -> Response<Body> {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header("Content-Type", "application/json")
        .body(Body::from(format!(r#"{{"error": "{}"}}"#, message)))
        .unwrap()
}

fn create_rate_limit_response(
    result: crate::types::BarnacleResult,
    config: &BarnacleConfig,
) -> Response<Body> {
    let retry_after = result.retry_after.unwrap_or(config.window);

    let mut builder = Response::builder()
        .status(StatusCode::TOO_MANY_REQUESTS)
        .header("Content-Type", "application/json")
        .header("X-RateLimit-Remaining", result.remaining.to_string())
        .header("Retry-After", retry_after.as_secs().to_string());

    if let Some(retry_after) = result.retry_after {
        builder = builder.header("X-RateLimit-Reset", retry_after.as_secs().to_string());
    }

    builder
        .body(Body::from(format!(
            r#"{{"error": "Rate limit exceeded", "retry_after": {}, "remaining": {}}}"#,
            retry_after.as_secs(),
            result.remaining
        )))
        .unwrap()
}

async fn handle_rate_limit_reset<S>(
    store: &Arc<S>,
    config: &BarnacleConfig,
    key: &BarnacleKey,
    status_code: u16,
) where
    S: BarnacleStore + 'static,
{
    if config.is_success_status(status_code) {
        if let Err(e) = store.reset(key).await {
            tracing::warn!("Failed to reset rate limit for key {:?}: {}", key, e);
        } else {
            tracing::debug!(
                "Reset rate limit for key {:?} after successful response",
                key
            );
        }
    }
}

pub fn create_api_key_layer<A, S>(api_key_store: A, rate_limit_store: S) -> ApiKeyLayer<A, S>
where
    A: ApiKeyStore + 'static,
    S: BarnacleStore + 'static,
{
    ApiKeyLayer::new(
        Arc::new(api_key_store),
        Arc::new(rate_limit_store),
        ApiKeyMiddlewareConfig::default(),
    )
}

pub fn create_api_key_layer_with_config<A, S>(
    api_key_store: A,
    rate_limit_store: S,
    config: ApiKeyMiddlewareConfig,
) -> ApiKeyLayer<A, S>
where
    A: ApiKeyStore + 'static,
    S: BarnacleStore + 'static,
{
    ApiKeyLayer::new(Arc::new(api_key_store), Arc::new(rate_limit_store), config)
}
