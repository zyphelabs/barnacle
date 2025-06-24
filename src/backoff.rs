use std::time::Duration;

/// Calculates the next wait time for exponential backoff
pub fn next_backoff(attempt: u32, backoff: &[Duration]) -> Option<Duration> {
    backoff.get(attempt as usize).cloned()
}
