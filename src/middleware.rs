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
use std::future::Future;
use tracing::debug;
use std::pin::Pin;

use crate::types::{ApiKeyConfig, ResetOnSuccess, NO_KEY};
use crate::RedisBarnacleStore;
use crate::{
    types::{BarnacleConfig, BarnacleContext, BarnacleKey},
    BarnacleStore,
};
use crate::error::BarnacleError;

/// Trait to extract the key from any payload type
pub trait KeyExtractable {
    fn extract_key(&self, request_parts: &Parts) -> BarnacleKey;
}

/// Error type for BarnacleLayerBuilder
#[derive(Debug, thiserror::Error)]
pub enum BarnacleLayerBuilderError {
    #[error("Missing store")]
    MissingStore,
    #[error("Missing config")]
    MissingConfig,
}

/// Builder for BarnacleLayer
pub struct BarnacleLayerBuilder<T = (), S = RedisBarnacleStore, State = (), E = BarnacleError, V = ()> {
    store: Option<S>,
    config: Option<BarnacleConfig>,
    state: Option<State>,
    api_key_validator: Option<V>,
    api_key_middleware_config: Option<ApiKeyConfig>,
    _phantom: PhantomData<(T, E)>,
}

impl<T, S, State, E, V> BarnacleLayerBuilder<T, S, State, E, V>
where
    S: BarnacleStore + 'static,
    State: Clone +Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    pub fn with_store(mut self, store: S) -> Self {
        self.store = Some(store);
        self
    }
    pub fn with_config(mut self, config: BarnacleConfig) -> Self {
        self.config = Some(config);
        self
    }
    pub fn with_state(mut self, state: State) -> Self {
        self.state = Some(state);
        self
    }
    pub fn with_api_key_validator(mut self, validator: V) -> Self {
        self.api_key_validator = Some(validator);
        self
    }
    pub fn with_api_key_middleware_config(mut self, config: ApiKeyConfig) -> Self {
        self.api_key_middleware_config = Some(config);
        self
    }
    pub fn build(self) -> Result<BarnacleLayer<T, S, State, E, V>, BarnacleLayerBuilderError> {
        Ok(BarnacleLayer {
            store: self.store.ok_or(BarnacleLayerBuilderError::MissingStore)?,
            config: self.config.ok_or(BarnacleLayerBuilderError::MissingConfig)?,
            state: self.state,
            api_key_validator: self.api_key_validator,
            api_key_middleware_config: self.api_key_middleware_config,
            _phantom: PhantomData,
        })
    }
}

/// Generic rate limiting and API key layer
pub struct BarnacleLayer<T = (), S = RedisBarnacleStore, State = (), E = BarnacleError, V = ()> {
    store: S,
    config: BarnacleConfig,
    state: Option<State>,
    api_key_validator: Option<V>,
    api_key_middleware_config: Option<ApiKeyConfig>,
    _phantom: PhantomData<(T, E)>,
}

impl<T, S, State, E, V> Clone for BarnacleLayer<T, S, State, E, V>
where
    S: Clone + BarnacleStore + 'static,
    State: Clone + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            config: self.config.clone(),
            state: self.state.clone(),
            api_key_validator: self.api_key_validator.clone(),
            api_key_middleware_config: self.api_key_middleware_config.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<T, S, State, E, V> BarnacleLayer<T, S, State, E, V>
where
    S: BarnacleStore + 'static,
    State: Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    pub fn builder() -> BarnacleLayerBuilder<T, S, State, E, V> {
        BarnacleLayerBuilder {
            store: None,
            config: None,
            state: None,
            api_key_validator: None,
            api_key_middleware_config: None,
            _phantom: PhantomData,
        }
    }
}

impl<Inner, T, S, State, E, V> Layer<Inner> for BarnacleLayer<T, S, State, E, V>
where
    T: DeserializeOwned + KeyExtractable + Send + 'static,
    S: Clone + BarnacleStore + 'static,
    State: Clone + Send + Sync + 'static,
    E: IntoResponse + Send + Sync + 'static,
    Inner: Clone,
    V: Clone + Send + Sync + 'static,
{
    type Service = BarnacleMiddleware<Inner, T, S, State, E, V>;
    fn layer(&self, inner: Inner) -> Self::Service {
        BarnacleMiddleware {
            inner,
            store: self.store.clone(),
            config: self.config.clone(),
            state: self.state.clone(),
            api_key_validator: self.api_key_validator.clone(),
            api_key_config: self.api_key_middleware_config.clone(),
            _phantom: PhantomData,
        }
    }
}

/// Helper function to handle rate limit reset logic
async fn handle_rate_limit_reset<S>(
    store: &S,
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
        debug!(
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
            Ok(_) => debug!(
                "Rate limit reset for {} {:?} after successful request (status: {}) path: {}",
                key_type,
                ctx.key,
                status_code,
                ctx.path
            ),
            Err(e) => debug!(
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
        debug!("IP via ConnectInfo: {}", addr.ip());
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
    debug!("Local key: {}", local_key);
    BarnacleKey::Ip(local_key)
}



/// The actual middleware that handles payload-based key extraction
pub struct BarnacleMiddleware<Inner, T, S, State = (), E = BarnacleError, V = ()> {
    inner: Inner,
    store: S,
    config: BarnacleConfig,
    state: Option<State>,
    api_key_validator: Option<V>,
    api_key_config: Option<ApiKeyConfig>,
    _phantom: PhantomData<(T, E)>,
}

impl<Inner, T, S, State, E, V> Clone for BarnacleMiddleware<Inner, T, S, State, E, V>
where
    Inner: Clone,
    S: Clone + BarnacleStore + 'static,
    State: Clone + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            store: self.store.clone(),
            config: self.config.clone(),
            state: self.state.clone(),
            api_key_validator: self.api_key_validator.clone(),
            api_key_config: self.api_key_config.clone(),
            _phantom: PhantomData,
        }
    }
}

// --- ValidatorCall trait for owned types ---
pub trait ValidatorCall<T, S, State, E> {
    fn call(
        &self,
        api_key: T,
        api_key_config: S,
        parts: Arc<Parts>,
        state: State,
    ) -> Pin<Box<dyn Future<Output = Result<(), E>> + Send>>;
}

// Implementation for closures
impl<F, Fut, T, S, State, E> ValidatorCall<T, S, State, E> for F
where
    F: Fn(T, S, Arc<Parts>, State) -> Fut + Send + Sync,
    Fut: Future<Output = Result<(), E>> + Send + 'static,
    T: Send + 'static,
    S: Send + 'static,
    State: Send + 'static,
    E: Send + 'static,
{
    fn call(
        &self,
        api_key: T,
        api_key_config: S,
        parts: Arc<Parts>,
        state: State,
    ) -> Pin<Box<dyn Future<Output = Result<(), E>> + Send>> {
        Box::pin((self)(api_key, api_key_config, parts, state))
    }
}

// Implementation for ()
impl<T, S, State, E> ValidatorCall<T, S, State, E> for () {
    fn call(
        &self,
        _api_key: T,
        _api_key_config: S,
        _parts: Arc<Parts>,
        _state: State,
    ) -> Pin<Box<dyn Future<Output = Result<(), E>> + Send>> {
        Box::pin(async { Ok(()) })
    }
}

// Provide a KeyExtractable impl for ()
impl KeyExtractable for () {
    fn extract_key(&self, request_parts: &Parts) -> BarnacleKey {
        // Use fallback key logic
        let extensions = &request_parts.extensions;
        let headers = &request_parts.headers;
        let path = request_parts.uri.path();
        let method = &request_parts.method;
        get_fallback_key_common(extensions, headers, path, method)
    }
}

impl<Inner, B, T, S, State, E, V> Service<Request<B>> for BarnacleMiddleware<Inner, T, S, State, E, V>
where
    Inner: Service<Request<axum::body::Body>, Response = Response<Body>> + Clone + Send + 'static,
    Inner::Future: Send + 'static,
    B: axum::body::HttpBody + Send + 'static,
    B::Data: Send,
    B::Error: std::error::Error + Send + Sync,
    S: Clone + BarnacleStore + 'static,
    State: Clone + Send + Sync + 'static,
    T: KeyExtractable + DeserializeOwned + Send + 'static,
    E: IntoResponse + Send + Sync + 'static + From<BarnacleError>,
    V: ValidatorCall<String, ApiKeyConfig, State, E> + Clone + Send + Sync + 'static,
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
        debug!("[middleware.rs] Unified BarnacleMiddleware::call invoked");
        let mut inner = self.inner.clone();
        let store = self.store.clone();
        let config = self.config.clone();
        let state = self.state.clone();
        let api_key_validator = self.api_key_validator.clone();
        let api_key_config = self.api_key_config.clone();
        Box::pin(async move {
            debug!("[middleware.rs] Entered async block in call");
            let current_path = req
                .extensions()
                .get::<OriginalUri>()
                .map(|original_url| original_url.path().to_owned())
                .unwrap_or(req.uri().path().to_owned());
            
            debug!("[middleware.rs] current_path: {}", current_path);
            let (parts, body) = req.into_parts();
            debug!("[middleware.rs] Request parts and body split");

            // API key validation (if configured)
            let mut api_key_used: Option<String> = None;
            let api_key_config = api_key_config.unwrap_or_default();
            let api_key = parts.headers.get(api_key_config.header_name.as_str()).and_then(|h| h.to_str().ok()).unwrap_or("");
            debug!("[middleware.rs] About to call validator with key: '{}'", api_key);

            let validation_result = if let Some(validator) = api_key_validator.as_ref() {
                let is_stateless_validator = std::any::TypeId::of::<V>() == std::any::TypeId::of::<()>();
                let is_unit_state = std::any::TypeId::of::<State>() == std::any::TypeId::of::<()>();
                if is_stateless_validator && is_unit_state {
                    // Both validator and state are (), safe to call with zeroed State
                    validator.call(api_key.to_string(), api_key_config, Arc::new(parts.clone()), unsafe { std::mem::zeroed() }).await
                } else {
                    match state {
                        Some(state) => {
                            validator.call(api_key.to_string(), api_key_config, Arc::new(parts.clone()), state).await
                        }
                        None => {
                            // Return a more appropriate error for missing validator state
                            Err(E::from(BarnacleError::custom("Barnacle: API key validator requires state, but none was provided. Use with_state() or use () for stateless validators.", None)))
                        }
                    }
                }
            } else {
                Ok(())
            };
            match validation_result {
                Ok(_) => {
                    debug!("[middleware.rs] Validator returned Ok for: '{}'", api_key);
                    if !api_key.is_empty() {
                        api_key_used = Some(api_key.to_string());
                    }
                },
                Err(e) => {
                    debug!("[middleware.rs] Validator returned Err");
                    return Ok(e.into_response());
                }
            }

            // Unified logic: always try to extract key from body (for T=(), uses fallback)
            let (rate_limit_context, body_bytes) = match body.collect().await {
                Ok(collected) => {
                    let bytes = collected.to_bytes();
                    let (key, used_fallback) = if let Some(ref api_key) = api_key_used {
                        // Use API key as the rate limiting key
                        (BarnacleKey::ApiKey(api_key.clone()), false)
                    } else {
                        match serde_json::from_slice::<T>(&bytes) {
                            Ok(payload) => (payload.extract_key(&parts), false),
                            Err(_) => (
                                get_fallback_key_common(
                                    &parts.extensions,
                                    &parts.headers,
                                    &current_path,
                                    &parts.method,
                                ),
                                true,
                            ),
                        }
                    };
                    let context = BarnacleContext {
                        key,
                        path: current_path.clone(),
                        method: parts.method.as_str().to_string(),
                    };
                    if used_fallback {
                        debug!("[middleware.rs] (unified) Using fallback key for rate limiting");
                    } else if api_key_used.is_some() {
                        debug!("[middleware.rs] (unified) Using API key for rate limiting");
                    } else {
                        debug!("[middleware.rs] (unified) Extracted key from payload for rate limiting");
                    }
                    (context, Some(bytes))
                }
                Err(_) => {
                    debug!("[middleware.rs] (unified) Failed to collect body, using fallback key");
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
                    (context, None)
                }
            };
            debug!("[middleware.rs] (unified) About to increment rate limit for context: {:?}", rate_limit_context);
            tracing::debug!("[middleware.rs] Rate limit increment: api_key={:?}, path={}, method={}", rate_limit_context.key, rate_limit_context.path, rate_limit_context.method);
            let result = match store.increment(&rate_limit_context, &config).await {
                Ok(result) => result,
                Err(e) => {
                    debug!("[middleware.rs] (unified) Rate limit store error: {}", e);
                    return Ok(E::from(e).into_response());
                }
            };
            debug!("[middleware.rs] (unified) Rate limit check passed for key: {:?}, remaining: {}, retry_after: {:?}", rate_limit_context.key, result.remaining, result.retry_after);
            let reconstructed_body = match body_bytes {
                Some(bytes) => axum::body::Body::from(bytes),
                None => axum::body::Body::empty(),
            };
            let new_req = Request::from_parts(parts, reconstructed_body);
            debug!("[middleware.rs] (unified) Calling inner service");
            let response = inner.call(new_req).await?;
            // Add rate limit headers to successful response
            let mut response_with_headers = response;
            {
                let headers = response_with_headers.headers_mut();
                if let Ok(remaining_header) = result.remaining.to_string().parse() {
                    headers.insert("X-RateLimit-Remaining", remaining_header);
                    debug!("[middleware.rs] (unified) Added X-RateLimit-Remaining: {}", result.remaining);
                }
                if let Ok(limit_header) = config.max_requests.to_string().parse() {
                    headers.insert("X-RateLimit-Limit", limit_header);
                    debug!("[middleware.rs] (unified) Added X-RateLimit-Limit: {}", config.max_requests);
                }
                if let Some(retry_after) = result.retry_after {
                    if let Ok(reset_header) = retry_after.as_secs().to_string().parse() {
                        headers.insert("X-RateLimit-Reset", reset_header);
                        debug!("[middleware.rs] (unified) Added X-RateLimit-Reset: {}", retry_after.as_secs());
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
            debug!("[middleware.rs] (unified) Returning final response");
            Ok(response_with_headers)
        })
    }
}
