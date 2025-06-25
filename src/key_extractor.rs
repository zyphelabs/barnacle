use axum::extract::Request;
use http_body_util::BodyExt;
use serde::de::DeserializeOwned;
use std::marker::PhantomData;
use std::sync::Arc;
use std::task::{Context, Poll};
use tower::{Layer, Service};

use crate::{
    BarnacleStore,
    types::{BarnacleConfig, BarnacleKey},
};

/// Trait to extract the key from any payload type
pub trait KeyExtractable {
    fn extract_key(&self) -> BarnacleKey;
}

/// Generic rate limiting layer that can extract keys from request bodies
pub struct GenericRateLimitLayer<T, S> {
    store: Arc<S>,
    config: BarnacleConfig,
    _phantom: PhantomData<T>,
}

impl<T, S> Clone for GenericRateLimitLayer<T, S> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            config: self.config.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<T, S> GenericRateLimitLayer<T, S>
where
    S: BarnacleStore + 'static,
{
    pub fn new(store: Arc<S>, config: BarnacleConfig) -> Self {
        Self {
            store,
            config,
            _phantom: PhantomData,
        }
    }
}

impl<Inner, T, S> Layer<Inner> for GenericRateLimitLayer<T, S>
where
    T: DeserializeOwned + KeyExtractable + Send + 'static,
    S: BarnacleStore + 'static,
{
    type Service = GenericRateLimitService<Inner, T, S>;

    fn layer(&self, inner: Inner) -> Self::Service {
        GenericRateLimitService {
            inner,
            store: self.store.clone(),
            config: self.config.clone(),
            _phantom: PhantomData,
        }
    }
}

pub struct GenericRateLimitService<Inner, T, S> {
    inner: Inner,
    store: Arc<S>,
    config: BarnacleConfig,
    _phantom: PhantomData<T>,
}

impl<Inner, T, S> Clone for GenericRateLimitService<Inner, T, S>
where
    Inner: Clone,
    S: BarnacleStore + 'static,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            store: self.store.clone(),
            config: self.config.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<Inner, B, T, S> Service<Request<B>> for GenericRateLimitService<Inner, T, S>
where
    Inner: Service<Request<axum::body::Body>> + Clone + Send + 'static,
    Inner::Future: Send + 'static,
    B: axum::body::HttpBody + Send + 'static,
    B::Data: Send,
    B::Error: std::error::Error + Send + Sync,
    T: DeserializeOwned + KeyExtractable + Send + 'static,
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
        let store = self.store.clone();
        let config = self.config.clone();

        Box::pin(async move {
            let (parts, body) = req.into_parts();

            // Extract body bytes
            let body_bytes = match body.collect().await {
                Ok(collected) => collected.to_bytes(),
                Err(_) => {
                    // If we can't collect the body, use IP fallback and continue
                    let fallback_key = get_fallback_key(&parts);
                    let result = store.increment(&fallback_key, &config).await;

                    if !result.allowed {
                        println!(
                            "[GenericRateLimit] Rate limit exceeded for fallback key: {:?}",
                            fallback_key
                        );
                    }

                    let req = Request::from_parts(parts, axum::body::Body::empty());
                    return inner.call(req).await;
                }
            };

            // Try to parse payload for rate limiting
            let rate_limit_key = if let Ok(payload) = serde_json::from_slice::<T>(&body_bytes) {
                payload.extract_key()
            } else {
                // If parsing fails, fall back to IP-based rate limiting
                get_fallback_key(&parts)
            };

            println!(
                "[GenericRateLimit] Checking rate limit for key: {:?}",
                rate_limit_key
            );

            // Check rate limit
            let result = store.increment(&rate_limit_key, &config).await;

            println!("[GenericRateLimit] Rate limit result: {:?}", result);

            if !result.allowed {
                println!(
                    "[GenericRateLimit] Rate limit exceeded - request will continue but rate limit was applied"
                );
            }

            // Reconstruct request with original body
            let new_body = axum::body::Body::from(body_bytes);
            let new_req = Request::from_parts(parts, new_body);

            inner.call(new_req).await
        })
    }
}

fn get_fallback_key(parts: &axum::http::request::Parts) -> BarnacleKey {
    // 1. Try X-Forwarded-For header
    if let Some(forwarded) = parts.headers.get("x-forwarded-for") {
        if let Ok(forwarded) = forwarded.to_str() {
            let ip = forwarded.split(',').next().unwrap_or("").trim();
            if !ip.is_empty() && ip != "unknown" {
                return BarnacleKey::Ip(ip.to_string());
            }
        }
    }

    // 2. Try X-Real-IP header
    if let Some(real_ip) = parts.headers.get("x-real-ip") {
        if let Ok(real_ip) = real_ip.to_str() {
            if !real_ip.is_empty() && real_ip != "unknown" {
                return BarnacleKey::Ip(real_ip.to_string());
            }
        }
    }

    // 3. For local/unknown requests, use route + method
    let path = parts.uri.path();
    let method = parts.method.as_str();
    let local_key = format!("fallback:{}:{}", method, path);
    BarnacleKey::Ip(local_key)
}

/// Helper function to create the generic rate limit layer with type inference
pub fn create_generic_rate_limit_layer<T, S>(
    store: Arc<S>,
    config: BarnacleConfig,
) -> GenericRateLimitLayer<T, S>
where
    T: DeserializeOwned + KeyExtractable + Send + 'static,
    S: BarnacleStore + 'static,
{
    GenericRateLimitLayer::new(store, config)
}
