use std::time::Duration;

/// Rate limiter configuration
#[derive(Clone, Debug)]
pub struct BarnacleConfig {
    pub max_requests: u32,
    pub window: Duration,
    pub backoff: Option<Vec<Duration>>, // Optional exponential
    pub reset_on_success: bool,
}

impl Default for BarnacleConfig {
    fn default() -> Self {
        Self {
            max_requests: 20,
            window: Duration::from_secs(60), // 1 minute
            backoff: None,
            reset_on_success: false,
        }
    }
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
