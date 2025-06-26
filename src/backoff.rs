use std::time::Duration;

/// Default exponential backoff sequence for failed login attempts
pub const DEFAULT_LOGIN_BACKOFF: &[Duration] = &[
    Duration::from_secs(10),  // First failure: 10 seconds
    Duration::from_secs(15),  // Second failure: 15 seconds
    Duration::from_secs(30),  // Third failure: 30 seconds
    Duration::from_secs(60),  // Fourth failure: 1 minute
    Duration::from_secs(120), // Fifth failure: 2 minutes
    Duration::from_secs(300), // Sixth+ failure: 5 minutes
];

/// Default exponential backoff sequence for API rate limiting
pub const DEFAULT_API_BACKOFF: &[Duration] = &[
    Duration::from_secs(60),   // First limit: 1 minute
    Duration::from_secs(180),  // Second limit: 3 minutes
    Duration::from_secs(600),  // Third limit: 10 minutes
    Duration::from_secs(1800), // Fourth+ limit: 30 minutes
];

/// Calculates the next wait time for exponential backoff
pub fn next_backoff(attempt: u32, backoff: &[Duration]) -> Option<Duration> {
    backoff
        .get(attempt as usize)
        .cloned()
        .or_else(|| backoff.last().cloned()) // Use last value if attempt exceeds sequence
}

/// Gets the appropriate backoff duration for login attempts
pub fn get_login_backoff(failed_attempts: u32) -> Duration {
    next_backoff(failed_attempts.saturating_sub(1), DEFAULT_LOGIN_BACKOFF).unwrap_or_else(|| {
        // This should never happen since DEFAULT_LOGIN_BACKOFF is non-empty
        // but we provide a fallback for safety
        DEFAULT_LOGIN_BACKOFF[DEFAULT_LOGIN_BACKOFF.len().saturating_sub(1)]
    })
}

/// Gets the appropriate backoff duration for API rate limiting
pub fn get_api_backoff(violation_count: u32) -> Duration {
    next_backoff(violation_count.saturating_sub(1), DEFAULT_API_BACKOFF).unwrap_or_else(|| {
        // This should never happen since DEFAULT_API_BACKOFF is non-empty
        // but we provide a fallback for safety
        DEFAULT_API_BACKOFF[DEFAULT_API_BACKOFF.len().saturating_sub(1)]
    })
}

/// Creates a custom backoff sequence
pub fn create_custom_backoff(
    base_seconds: u64,
    multiplier: f64,
    max_attempts: usize,
) -> Vec<Duration> {
    let mut backoff = Vec::with_capacity(max_attempts);
    let mut current = base_seconds as f64;

    for _ in 0..max_attempts {
        backoff.push(Duration::from_secs(current as u64));
        current *= multiplier;
    }

    backoff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_login_backoff_progression() {
        assert_eq!(get_login_backoff(1), Duration::from_secs(10));
        assert_eq!(get_login_backoff(2), Duration::from_secs(15));
        assert_eq!(get_login_backoff(3), Duration::from_secs(30));
        assert_eq!(get_login_backoff(10), Duration::from_secs(300)); // Max value
    }

    #[test]
    fn test_api_backoff_progression() {
        assert_eq!(get_api_backoff(1), Duration::from_secs(60));
        assert_eq!(get_api_backoff(2), Duration::from_secs(180));
        assert_eq!(get_api_backoff(10), Duration::from_secs(1800)); // Max value
    }

    #[test]
    fn test_custom_backoff() {
        let custom = create_custom_backoff(5, 2.0, 4);
        assert_eq!(custom.len(), 4);
        assert_eq!(custom[0], Duration::from_secs(5));
        assert_eq!(custom[1], Duration::from_secs(10));
        assert_eq!(custom[2], Duration::from_secs(20));
        assert_eq!(custom[3], Duration::from_secs(40));
    }
}
