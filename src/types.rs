use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub enum ResetOnSuccess {
    Not,
    Yes(Option<Vec<u16>>),
}

/// Rate limiter configuration
#[derive(Clone, Debug)]
pub struct BarnacleConfig {
    pub max_requests: u32,
    pub window: Duration,
    pub reset_on_success: ResetOnSuccess,
}

impl Default for BarnacleConfig {
    fn default() -> Self {
        Self {
            max_requests: 20,
            window: Duration::from_secs(60), // 1 minute
            reset_on_success: ResetOnSuccess::Not,
        }
    }
}

impl BarnacleConfig {
    /// Check if a status code should be considered successful for rate limit reset
    pub fn is_success_status(&self, status_code: u16) -> bool {
        match &self.reset_on_success {
            ResetOnSuccess::Not => false,
            ResetOnSuccess::Yes(success_codes) => {
                if let Some(codes) = success_codes {
                    codes.contains(&status_code)
                } else {
                    // Default to 2xx status codes
                    status_code >= 200 && status_code < 300
                }
            }
        }
    }
}

/// Identification key for rate limiting (e.g., email, api-key, IP)
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum BarnacleKey {
    Email(String),
    ApiKey(String),
    Ip(String),
    Custom(String),
}

/// Result of an increment attempt
#[derive(Clone, Debug)]
pub struct BarnacleResult {
    pub allowed: bool,
    pub remaining: u32,
    pub retry_after: Option<Duration>,
}
