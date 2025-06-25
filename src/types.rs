use std::time::Duration;

/// Rate limiter configuration
#[derive(Clone, Debug)]
pub struct BarnacleConfig {
    pub max_requests: u32,
    pub window: Duration,
    pub backoff: Option<Vec<Duration>>, // Optional exponential
    pub reset_on_success: bool,
    /// HTTP status codes that are considered successful for resetting rate limits
    /// Defaults to 2xx status codes if not specified
    pub success_status_codes: Option<Vec<u16>>,
}

impl Default for BarnacleConfig {
    fn default() -> Self {
        Self {
            max_requests: 20,
            window: Duration::from_secs(60), // 1 minute
            backoff: None,
            reset_on_success: false,
            success_status_codes: None,
        }
    }
}

impl BarnacleConfig {
    /// Check if a status code should be considered successful for rate limit reset
    pub fn is_success_status(&self, status_code: u16) -> bool {
        if let Some(ref success_codes) = self.success_status_codes {
            success_codes.contains(&status_code)
        } else {
            // Default to 2xx status codes
            status_code >= 200 && status_code < 300
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
