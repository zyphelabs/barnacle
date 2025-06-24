use std::time::Duration;

/// Rate limiter configuration
#[derive(Clone, Debug)]
pub struct BarnacleConfig {
    pub max_requests: u32,
    pub window: Duration,
    pub backoff: Option<Vec<Duration>>, // Optional exponential
    pub reset_on_success: bool,
}

/// Identification key for rate limiting (e.g., email, api-key, IP)
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum BarnacleKey {
    Email(String),
    ApiKey(String),
    Ip(String),
}

/// Result of an increment attempt
#[derive(Clone, Debug)]
pub struct BarnacleResult {
    pub allowed: bool,
    pub remaining: u32,
    pub retry_after: Option<Duration>,
}
