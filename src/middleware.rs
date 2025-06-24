use std::sync::Arc;
use std::task::{Context, Poll};

use axum::{body::Body, http::Request, response::Response};
use tower::Layer;
use tower::Service;

use crate::{BarnacleStore, types::BarnacleConfig};

/// Rate limiting middleware for Axum
pub struct BarnacleLayer<S: BarnacleStore + 'static> {
    store: Arc<S>,
    config: BarnacleConfig,
}

impl<S: BarnacleStore + 'static> BarnacleLayer<S> {
    pub fn new(store: Arc<S>, config: BarnacleConfig) -> Self {
        Self { store, config }
    }
}

impl<S: BarnacleStore + 'static> Clone for BarnacleLayer<S> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            config: self.config.clone(),
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
        }
    }
}

/// The actual middleware
pub struct BarnacleMiddleware<S: BarnacleStore + 'static, Inner> {
    inner: Inner,
    store: Arc<S>,
    config: BarnacleConfig,
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

        Box::pin(async move {
            // Extract rate limit key from request (IP address for now)
            let key = crate::types::BarnacleKey::Ip(
                req.extensions()
                    .get::<std::net::SocketAddr>()
                    .map(|addr| addr.ip().to_string())
                    .unwrap_or_else(|| "unknown".to_string()),
            );

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
                    .body(Body::from("Rate limit exceeded"))
                    .unwrap();

                return Ok(response);
            }

            // Continue with the request
            inner.call(req).await
        })
    }
}
