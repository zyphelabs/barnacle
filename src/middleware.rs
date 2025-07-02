use axum::body::Body;
use axum::extract::Request;
use axum::http::Response;
use http_body_util::BodyExt;
use serde::de::DeserializeOwned;
use std::marker::PhantomData;
use std::sync::Arc;
use std::task::{Context, Poll};
use tower::{Layer, Service};

use crate::types::ResetOnSuccess;
use crate::{
    types::{BarnacleConfig, BarnacleContext, BarnacleKey},
    BarnacleStore,
};

/// Trait to extract the key from any payload type
pub trait KeyExtractable {
    fn extract_key(&self) -> BarnacleKey;
}

/// Generic rate limiting layer that can extract keys from request bodies
pub struct BarnacleLayer<T, S> {
    store: Arc<S>,
    config: BarnacleConfig,
    _phantom: PhantomData<T>,
}

impl<T, S> Clone for BarnacleLayer<T, S> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            config: self.config.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<T, S> BarnacleLayer<T, S>
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

impl<Inner, T, S> Layer<Inner> for BarnacleLayer<T, S>
where
    T: DeserializeOwned + KeyExtractable + Send + 'static,
    S: BarnacleStore + 'static,
{
    type Service = BarnacleMiddleware<Inner, T, S>;

    fn layer(&self, inner: Inner) -> Self::Service {
        BarnacleMiddleware {
            inner,
            store: self.store.clone(),
            config: self.config.clone(),
            _phantom: PhantomData,
        }
    }
}

// Special implementation for () type that doesn't require KeyExtractable
impl<Inner, S> Layer<Inner> for BarnacleLayer<(), S>
where
    S: BarnacleStore + 'static,
{
    type Service = BarnacleMiddleware<Inner, (), S>;

    fn layer(&self, inner: Inner) -> Self::Service {
        BarnacleMiddleware {
            inner,
            store: self.store.clone(),
            config: self.config.clone(),
            _phantom: PhantomData,
        }
    }
}

/// The actual middleware that handles payload-based key extraction
pub struct BarnacleMiddleware<Inner, T, S> {
    inner: Inner,
    store: Arc<S>,
    config: BarnacleConfig,
    _phantom: PhantomData<T>,
}

impl<Inner, T, S> Clone for BarnacleMiddleware<Inner, T, S>
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

impl<Inner, B, T, S> Service<Request<B>> for BarnacleMiddleware<Inner, T, S>
where
    Inner: Service<Request<axum::body::Body>, Response = Response<Body>> + Clone + Send + 'static,
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

            // If T is (), we don't need to deserialize the body
            let (rate_limit_context, body_bytes) = if std::any::TypeId::of::<T>()
                == std::any::TypeId::of::<()>()
            {
                let fallback_key = get_fallback_key_from_parts(&parts);
                let context = BarnacleContext {
                    key: fallback_key,
                    path: parts.uri.path().to_string(),
                    method: parts.method.as_str().to_string(),
                };
                (context, None)
            } else {
                // Try to extract key from request body using KeyExtractable trait
                match body.collect().await {
                    Ok(collected) => {
                        let bytes = collected.to_bytes();
                        if let Ok(payload) = serde_json::from_slice::<T>(&bytes) {
                            let key = payload.extract_key();
                            let context = BarnacleContext {
                                key,
                                path: parts.uri.path().to_string(),
                                method: parts.method.as_str().to_string(),
                            };
                            (context, Some(bytes))
                        } else {
                            let fallback_key = get_fallback_key_from_parts(&parts);
                            let context = BarnacleContext {
                                key: fallback_key,
                                path: parts.uri.path().to_string(),
                                method: parts.method.as_str().to_string(),
                            };
                            (context, Some(bytes))
                        }
                    }
                    Err(_) => {
                        let fallback_key = get_fallback_key_from_parts(&parts);
                        let context = BarnacleContext {
                            key: fallback_key,
                            path: parts.uri.path().to_string(),
                            method: parts.method.as_str().to_string(),
                        };

                        let result = store.increment(&context, &config).await;
                        if !result.allowed {
                            let retry_after_secs =
                                result.retry_after.map(|d| d.as_secs()).unwrap_or(0);
                            tracing::warn!(
                                "Rate limit exceeded for fallback key: {:?}, retry after {} seconds",
                                context.key,
                                retry_after_secs
                            );
                            return Ok(create_rate_limit_response(result, &config));
                        }

                        tracing::debug!(
                            "Rate limit check passed for fallback key: {:?}, remaining: {}, retry_after: {:?}",
                            context.key,
                            result.remaining,
                            result.retry_after
                        );

                        let req = Request::from_parts(parts, axum::body::Body::empty());
                        let response = inner.call(req).await?;

                        handle_rate_limit_reset(
                            &store,
                            &config,
                            &context,
                            response.status().as_u16(),
                            true,
                        )
                        .await;

                        return Ok(response);
                    }
                }
            };

            let result = store.increment(&rate_limit_context, &config).await;

            if !result.allowed {
                let retry_after_secs = result.retry_after.map(|d| d.as_secs()).unwrap_or(0);
                tracing::debug!(
                    "Rate limit exceeded for key: {:?}, retry after {} seconds",
                    rate_limit_context.key,
                    retry_after_secs
                );

                return Ok(create_rate_limit_response(result, &config));
            }

            tracing::debug!(
                "Rate limit check passed for key: {:?}, remaining: {}, retry_after: {:?}",
                rate_limit_context.key,
                result.remaining,
                result.retry_after
            );

            // Reconstruct the request
            let new_req = if let Some(bytes) = body_bytes {
                // For payload types, reconstruct with the body bytes
                let new_body = axum::body::Body::from(bytes);
                Request::from_parts(parts, new_body)
            } else {
                // For () type, we don't need to reconstruct the body
                Request::from_parts(parts, axum::body::Body::empty())
            };

            let response = inner.call(new_req).await?;

            // Add rate limit headers to successful response
            let mut response_with_headers = response;
            let headers = response_with_headers.headers_mut();

            tracing::debug!(
                "Adding rate limit headers - remaining: {}, limit: {}",
                result.remaining,
                config.max_requests
            );

            if let Ok(remaining_header) = result.remaining.to_string().parse() {
                headers.insert("X-RateLimit-Remaining", remaining_header);
            }
            if let Ok(limit_header) = config.max_requests.to_string().parse() {
                headers.insert("X-RateLimit-Limit", limit_header);
            }
            if let Some(retry_after) = result.retry_after {
                if let Ok(reset_header) = retry_after.as_secs().to_string().parse() {
                    headers.insert("X-RateLimit-Reset", reset_header);
                    tracing::debug!("Added X-RateLimit-Reset: {}", retry_after.as_secs());
                }
            }

            handle_rate_limit_reset(
                &store,
                &config,
                &rate_limit_context,
                response_with_headers.status().as_u16(),
                false,
            )
            .await;

            Ok(response_with_headers)
        })
    }
}

// Special implementation for () type that doesn't require KeyExtractable
impl<Inner, B, S> Service<Request<B>> for BarnacleMiddleware<Inner, (), S>
where
    Inner: Service<Request<axum::body::Body>, Response = Response<Body>> + Clone + Send + 'static,
    Inner::Future: Send + 'static,
    B: axum::body::HttpBody + Send + 'static,
    B::Data: Send,
    B::Error: std::error::Error + Send + Sync,
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
            let (parts, _body) = req.into_parts();

            // For () type, always use fallback key
            let rate_limit_key = get_fallback_key_from_parts(&parts);
            let context = BarnacleContext {
                key: rate_limit_key,
                path: parts.uri.path().to_string(),
                method: parts.method.as_str().to_string(),
            };
            let result = store.increment(&context, &config).await;

            if !result.allowed {
                let retry_after_secs = result.retry_after.map(|d| d.as_secs()).unwrap_or(0);
                tracing::debug!(
                    "Rate limit exceeded for key: {:?}, retry after {} seconds",
                    context.key,
                    retry_after_secs
                );

                return Ok(create_rate_limit_response(result, &config));
            }

            tracing::debug!(
                "Rate limit check passed for key: {:?}, remaining: {}, retry_after: {:?}",
                context.key,
                result.remaining,
                result.retry_after
            );

            // For () type, we don't need to reconstruct the body
            let new_req = Request::from_parts(parts, axum::body::Body::empty());

            let response = inner.call(new_req).await?;

            // Add rate limit headers to successful response
            let mut response_with_headers = response;
            let headers = response_with_headers.headers_mut();

            if let Ok(remaining_header) = result.remaining.to_string().parse() {
                headers.insert("X-RateLimit-Remaining", remaining_header);
                tracing::debug!("Added X-RateLimit-Remaining: {}", result.remaining);
            }

            if let Some(retry_after) = result.retry_after {
                if let Ok(reset_header) = retry_after.as_secs().to_string().parse() {
                    headers.insert("X-RateLimit-Reset", reset_header);
                    tracing::debug!("Added X-RateLimit-Reset: {}", retry_after.as_secs());
                }
            }

            handle_rate_limit_reset(
                &store,
                &config,
                &context,
                response_with_headers.status().as_u16(),
                false,
            )
            .await;

            Ok(response_with_headers)
        })
    }
}

/// Helper function to create a rate limit exceeded response
fn create_rate_limit_response(
    result: crate::types::BarnacleResult,
    config: &BarnacleConfig,
) -> Response<Body> {
    let retry_after = result
        .retry_after
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|| "60".to_string());

    Response::builder()
        .status(429)
        .header("Retry-After", retry_after)
        .header("X-RateLimit-Remaining", "0")
        .header("X-RateLimit-Limit", config.max_requests.to_string())
        .body(Body::from("Rate limit exceeded"))
        .expect("Failed to build rate limit response")
}

/// Helper function to handle rate limit reset logic
async fn handle_rate_limit_reset<S>(
    store: &Arc<S>,
    config: &BarnacleConfig,
    context: &BarnacleContext,
    status_code: u16,
    is_fallback: bool,
) where
    S: BarnacleStore + 'static,
{
    if config.reset_on_success == ResetOnSuccess::Not {
        return;
    }

    let key_type = if is_fallback { "fallback key" } else { "key" };
    if !config.is_success_status(status_code) {
        tracing::debug!(
            "Not resetting rate limit for {} {:?} due to error status: {}",
            key_type,
            context.key,
            status_code
        );
        return;
    }

    match store.reset(context).await {
        Ok(_) => tracing::trace!(
            "Rate limit reset for {} {:?} after successful request (status: {})",
            key_type,
            context.key,
            status_code
        ),
        Err(e) => tracing::error!(
            "Failed to reset rate limit for {} {:?}: {}",
            key_type,
            context.key,
            e
        ),
    }
}

fn get_fallback_key_from_parts(parts: &axum::http::request::Parts) -> BarnacleKey {
    get_fallback_key_common(&parts.extensions, &parts.headers, &parts.uri, &parts.method)
}

fn get_fallback_key_common(
    extensions: &axum::http::Extensions,
    headers: &axum::http::HeaderMap,
    uri: &axum::http::Uri,
    method: &axum::http::Method,
) -> BarnacleKey {
    // 1. Try ConnectInfo<SocketAddr> (only available in full Request)
    if let Some(addr) = extensions.get::<axum::extract::ConnectInfo<std::net::SocketAddr>>() {
        tracing::trace!("IP via ConnectInfo: {}", addr.ip());
        return BarnacleKey::Ip(addr.ip().to_string());
    }

    // 2. Try X-Forwarded-For header
    if let Some(forwarded) = headers.get("x-forwarded-for") {
        if let Ok(forwarded) = forwarded.to_str() {
            let ip = forwarded.split(',').next().unwrap_or("").trim();
            if !ip.is_empty() && ip != "unknown" {
                return BarnacleKey::Ip(ip.to_string());
            }
        }
    }

    // 3. Try X-Real-IP header
    if let Some(real_ip) = headers.get("x-real-ip") {
        if let Ok(real_ip) = real_ip.to_str() {
            if !real_ip.is_empty() && real_ip != "unknown" {
                return BarnacleKey::Ip(real_ip.to_string());
            }
        }
    }

    // 4. For local requests, use a unique identifier based on route + method
    let path = uri.path();
    let method_str = method.as_str();
    let local_key = format!("local:{}:{}", method_str, path);
    BarnacleKey::Ip(local_key)
}

/// Helper function to create the barnacle layer for payload-based key extraction
pub fn create_barnacle_layer_for_payload<T>(
    store: impl BarnacleStore + 'static,
    config: BarnacleConfig,
) -> BarnacleLayer<T, impl BarnacleStore + 'static>
where
    T: DeserializeOwned + KeyExtractable + Send + 'static,
{
    BarnacleLayer::new(Arc::new(store), config)
}

/// Helper function to create the barnacle layer without payload deserialization
pub fn create_barnacle_layer(
    store: impl BarnacleStore + 'static,
    config: BarnacleConfig,
) -> BarnacleLayer<(), impl BarnacleStore + 'static> {
    BarnacleLayer::new(Arc::new(store), config)
}
