use std::sync::Arc;
use std::task::{Context, Poll};

use axum::{body::Body, http::Request, response::Response};
use tower::Layer;
use tower::Service;

use crate::{BarnacleStore, types::{BarnacleConfig, BarnacleKey}};

/// Rate limiting middleware for Axum
pub struct BarnacleLayer<S: BarnacleStore + 'static> {
    store: Arc<S>,
    config: BarnacleConfig,
    key_extractor: Arc<dyn Fn(&Request<Body>) -> Option<BarnacleKey> + Send + Sync>,
}

impl<S: BarnacleStore + 'static> BarnacleLayer<S> {
    pub fn new(
        store: Arc<S>,
        config: BarnacleConfig,
        key_extractor: Arc<dyn Fn(&Request<Body>) -> Option<BarnacleKey> + Send + Sync>,
    ) -> Self {
        Self { store, config, key_extractor }
    }
}

impl<S: BarnacleStore + 'static> Clone for BarnacleLayer<S> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            config: self.config.clone(),
            key_extractor: self.key_extractor.clone(),
        }
    }
}

impl<S, Inner> Layer<Inner> for BarnacleLayer<S>
where
    S: BarnacleStore + 'static,
{
    type Service = BarnacleMiddleware<S, Inner>;

    fn layer(&self, inner: Inner) -> Self::Service {
        BarnacleMiddleware {
            inner,
            store: self.store.clone(),
            config: self.config.clone(),
            key_extractor: self.key_extractor.clone(),
        }
    }
}

/// The actual middleware
pub struct BarnacleMiddleware<S: BarnacleStore + 'static, Inner> {
    inner: Inner,
    store: Arc<S>,
    config: BarnacleConfig,
    key_extractor: Arc<dyn Fn(&Request<Body>) -> Option<BarnacleKey> + Send + Sync>,
}

impl<S, Inner> Clone for BarnacleMiddleware<S, Inner>
where
    S: BarnacleStore + 'static,
    Inner: Clone,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            store: self.store.clone(),
            config: self.config.clone(),
            key_extractor: self.key_extractor.clone(),
        }
    }
}

impl<S, Inner> Service<Request<Body>> for BarnacleMiddleware<S, Inner>
where
    S: BarnacleStore + 'static,
    Inner: Service<Request<Body>, Response = Response> + Send + Clone + 'static,
    Inner::Future: Send + 'static,
{
    type Response = Inner::Response;
    type Error = Inner::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let store = self.store.clone();
        let config = self.config.clone();
        let mut inner = self.inner.clone();
        let key_extractor = self.key_extractor.clone();

        Box::pin(async move {
            let key = (key_extractor)(&req).unwrap_or_else(|| {
                // Fallback to IP
                BarnacleKey::Ip(
                    req.extensions()
                        .get::<std::net::SocketAddr>()
                        .map(|addr| addr.ip().to_string())
                        .unwrap_or_else(|| "unknown".to_string()),
                )
            });

            // Check rate limit
            let result = store.increment(&key, &config).await;

            if !result.allowed {
                // Return 429 Too Many Requests
                let response = Response::builder()
                    .status(429)
                    .header(
                        "Retry-After",
                        result
                            .retry_after
                            .map(|d| d.as_secs().to_string())
                            .unwrap_or_else(|| "60".to_string()),
                    )
                    .header("X-RateLimit-Remaining", "0")
                    .header("X-RateLimit-Limit", config.max_requests.to_string())
                    .body(Body::from("Rate limit exceeded"))
                    .unwrap();

                return Ok(response);
            }

            // Continue with the request and add rate limit headers
            let mut response = inner.call(req).await?;
            
            // Add rate limit headers to successful responses
            let headers = response.headers_mut();
            headers.insert("X-RateLimit-Remaining", result.remaining.to_string().parse().unwrap());
            headers.insert("X-RateLimit-Limit", config.max_requests.to_string().parse().unwrap());
            
            if let Some(retry_after) = result.retry_after {
                headers.insert("X-RateLimit-Reset", retry_after.as_secs().to_string().parse().unwrap());
            }

            Ok(response)
        })
    }
}

pub fn barnacle_layer_with_key_extractor<S: BarnacleStore + 'static>(
    store: Arc<S>,
    config: BarnacleConfig,
    key_extractor: Arc<dyn Fn(&Request<Body>) -> Option<BarnacleKey> + Send + Sync>,
) -> BarnacleLayer<S> {
    BarnacleLayer::new(store, config, key_extractor)
}
