use axum::body::Body;
use axum::extract::{OriginalUri, Request};
use axum::http::request::Parts;
use axum::http::Response;
use axum::response::IntoResponse;
use http_body_util::BodyExt;
use serde::de::DeserializeOwned;
use std::marker::PhantomData;
use std::sync::Arc;
use std::task::{Context, Poll};
use tower::{Layer, Service};

use crate::types::{ResetOnSuccess, NO_KEY};
use crate::{
    types::{BarnacleConfig, BarnacleContext, BarnacleKey},
    BarnacleStore,
};

/// Trait to extract the key from any payload type
pub trait KeyExtractable {
    fn extract_key(&self, request_parts: &Parts) -> BarnacleKey;
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
            let current_path = {
                let original_path = req
                    .extensions()
                    .get::<OriginalUri>()
                    .map(|original_url| original_url.path().to_owned());

                original_path.unwrap_or_else(|| req.uri().path().to_owned())
            };
            let (parts, body) = req.into_parts();

            // If T is (), we don't need to deserialize the body
            let (rate_limit_context, body_bytes) = if std::any::TypeId::of::<T>()
                == std::any::TypeId::of::<()>()
            {
                let fallback_key = get_fallback_key_common(
                    &parts.extensions,
                    &parts.headers,
                    &current_path,
                    &parts.method,
                );
                let context = BarnacleContext {
                    key: fallback_key,
                    path: current_path,
                    method: parts.method.as_str().to_string(),
                };
                (context, None)
            } else {
                // Try to extract key from request body using KeyExtractable trait
                match body.collect().await {
                    Ok(collected) => {
                        let bytes = collected.to_bytes();
                        if let Ok(payload) = serde_json::from_slice::<T>(&bytes) {
                            let key = payload.extract_key(&parts);
                            let context = BarnacleContext {
                                key,
                                path: current_path.clone(),
                                method: parts.method.as_str().to_string(),
                            };
                            (context, Some(bytes))
                        } else {
                            let fallback_key = get_fallback_key_common(
                                &parts.extensions,
                                &parts.headers,
                                &current_path,
                                &parts.method,
                            );
                            let context = BarnacleContext {
                                key: fallback_key,
                                path: current_path.clone(),
                                method: parts.method.as_str().to_string(),
                            };
                            (context, Some(bytes))
                        }
                    }
                    Err(_) => {
                        let fallback_key = get_fallback_key_common(
                            &parts.extensions,
                            &parts.headers,
                            &current_path,
                            &parts.method,
                        );
                        let context = BarnacleContext {
                            key: fallback_key,
                            path: current_path.clone(),
                            method: parts.method.as_str().to_string(),
                        };

                        let result = match store.increment(&context, &config).await {
                            Ok(result) => result,
                            Err(e) => {
                                tracing::debug!("Rate limit store error: {}", e);
                                return Ok(e.into_response());
                            }
                        };

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

            let result = match store.increment(&rate_limit_context, &config).await {
                Ok(result) => result,
                Err(e) => {
                    tracing::debug!("Rate limit store error: {}", e);
                    return Ok(e.into_response());
                }
            };

            tracing::debug!(
                "Rate limit check passed for key: {:?}, remaining: {}, retry_after: {:?}",
                rate_limit_context.key,
                result.remaining,
                result.retry_after
            );

            let reconstructed_body = match body_bytes {
                Some(bytes) => axum::body::Body::from(bytes),
                None => axum::body::Body::empty(),
            };

            let new_req = Request::from_parts(parts, reconstructed_body);

            let response = inner.call(new_req).await?;

            // Add rate limit headers to successful response
            let mut response_with_headers = response;
            {
                let headers = response_with_headers.headers_mut();
                if let Ok(remaining_header) = result.remaining.to_string().parse() {
                    headers.insert("X-RateLimit-Remaining", remaining_header);
                    tracing::debug!("Added X-RateLimit-Remaining: {}", result.remaining);
                }

                if let Ok(limit_header) = config.max_requests.to_string().parse() {
                    headers.insert("X-RateLimit-Limit", limit_header);
                    tracing::debug!("Added X-RateLimit-Limit: {}", config.max_requests);
                }

                if let Some(retry_after) = result.retry_after {
                    if let Ok(reset_header) = retry_after.as_secs().to_string().parse() {
                        headers.insert("X-RateLimit-Reset", reset_header);
                        tracing::debug!("Added X-RateLimit-Reset: {}", retry_after.as_secs());
                    }
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
            let current_path = {
                let original_path = req
                    .extensions()
                    .get::<OriginalUri>()
                    .map(|original_url| original_url.path().to_owned());

                original_path.unwrap_or_else(|| req.uri().path().to_owned())
            };
            let (parts, body) = req.into_parts();

            // For () type, always use fallback key
            let rate_limit_key = get_fallback_key_common(
                &parts.extensions,
                &parts.headers,
                &current_path,
                &parts.method,
            );
            let context = BarnacleContext {
                key: rate_limit_key,
                path: current_path,
                method: parts.method.as_str().to_string(),
            };
            let result = match store.increment(&context, &config).await {
                Ok(result) => result,
                Err(e) => {
                    tracing::debug!("Rate limit store error: {}", e);
                    return Ok(e.into_response());
                }
            };

            tracing::debug!(
                "Rate limit check passed for key: {:?}, remaining: {}, retry_after: {:?}",
                context,
                result.remaining,
                result.retry_after
            );

            // For () type, we need to preserve the original body
            let new_body = match body.collect().await {
                Ok(collected) => {
                    let bytes = collected.to_bytes();
                    axum::body::Body::from(bytes)
                }
                Err(_) => axum::body::Body::empty(),
            };
            let new_req = Request::from_parts(parts, new_body);

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

    let mut contexts = vec![context.clone()];

    if let ResetOnSuccess::Multiple(_, extra_contexts) = &config.reset_on_success {
        contexts.extend(extra_contexts.iter().cloned());
    }

    for ctx in contexts.iter_mut() {
        if ctx.key == BarnacleKey::Custom(NO_KEY.to_string()) {
            ctx.key = context.key.clone();
        }
        match store.reset(ctx).await {
            Ok(_) => tracing::trace!(
                "Rate limit reset for {} {:?} after successful request (status: {}) path: {}",
                key_type,
                ctx.key,
                status_code,
                ctx.path
            ),
            Err(e) => tracing::error!(
                "Failed to reset rate limit for {} {:?}: {} path: {}",
                key_type,
                ctx.key,
                e,
                ctx.path
            ),
        }
    }
}

fn get_fallback_key_common(
    extensions: &axum::http::Extensions,
    headers: &axum::http::HeaderMap,
    path: &str,
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
