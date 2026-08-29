//! Shared retry/backoff decision logic used by both the GitHub client and
//! Hacker News discovery. Deliberately dependency-free — no `rand` crate —
//! jitter is derived from the system clock, which is adequate for spacing
//! out retries and doesn't need to be cryptographically random.
//!
//! Split out as pure functions specifically so they're unit-testable
//! without any HTTP I/O or mocking: the policy (what to retry, how long to
//! wait) is decoupled from the mechanism (actually sending the request).

use reqwest::header::HeaderMap;
use reqwest::StatusCode;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::Duration;

const RETRY_BASE_DELAY_MS: u64 = 500;

/// Whether a response is worth retrying, given how many attempts have
/// already been made (1-indexed: `attempt` is the attempt that just
/// happened).
///
/// A plain 403 with no rate-limit signal is a permanent permission error
/// (private repo, bad token scope, blocked, ...) and is deliberately NOT
/// retried. Only a 403 carrying a `Retry-After` header (GitHub's secondary
/// / abuse-detection rate limit) or an exhausted
/// `X-RateLimit-Remaining: 0` (primary rate limit) is treated as
/// rate-limit-related and retried.
pub fn should_retry(
    status: StatusCode,
    headers: &HeaderMap,
    attempt: u32,
    max_retries: u32,
) -> bool {
    if attempt >= max_retries {
        return false;
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return true;
    }
    if status == StatusCode::FORBIDDEN {
        let secondary_limit_signal = headers.get("retry-after").is_some();
        let primary_limit_exhausted = headers
            .get("x-ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<i64>().ok())
            .is_some_and(|remaining| remaining == 0);
        return secondary_limit_signal || primary_limit_exhausted;
    }
    status.is_server_error()
}

/// How long to wait before the next attempt: honors a `Retry-After`
/// header (seconds) when present, else falls back to exponential backoff
/// with a little clock-derived jitter.
pub fn retry_delay(headers: &HeaderMap, attempt: u32) -> Duration {
    retry_after_from_headers(headers).unwrap_or_else(|| backoff_delay(attempt))
}

fn retry_after_from_headers(headers: &HeaderMap) -> Option<Duration> {
    headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
}

pub fn backoff_delay(attempt: u32) -> Duration {
    let exponent = attempt.saturating_sub(1).min(6);
    let base = Duration::from_millis(RETRY_BASE_DELAY_MS);
    let scaled = base.saturating_mul(1u32 << exponent);
    let jitter_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_millis() as u64 % 250)
        .unwrap_or(0);
    scaled + Duration::from_millis(jitter_ms)
}

/// Should we proactively pause before the *next* call because a service's
/// rate-limit window is nearly exhausted? Returns the number of seconds to
/// wait, capped at `max_wait_secs` so a stale or garbled reset timestamp
/// can't stall a run indefinitely — beyond the cap, the caller proceeds
/// anyway rather than blocking forever.
pub fn proactive_wait_secs(
    remaining: Option<i64>,
    reset_epoch: Option<i64>,
    floor: i64,
    max_wait_secs: i64,
) -> Option<i64> {
    let remaining = remaining?;
    let reset_epoch = reset_epoch?;
    if remaining > floor {
        return None;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let wait = (reset_epoch - now).clamp(0, max_wait_secs);
    if wait > 0 {
        Some(wait)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderName, HeaderValue};

    #[test]
    fn retryable_statuses_are_identified_correctly() {
        let empty = HeaderMap::new();
        assert!(should_retry(StatusCode::TOO_MANY_REQUESTS, &empty, 1, 3));
        assert!(should_retry(StatusCode::BAD_GATEWAY, &empty, 1, 3));
        assert!(should_retry(StatusCode::SERVICE_UNAVAILABLE, &empty, 1, 3));
        assert!(!should_retry(StatusCode::NOT_FOUND, &empty, 1, 3));
        assert!(!should_retry(StatusCode::OK, &empty, 1, 3));
    }

    #[test]
    fn plain_forbidden_with_no_rate_limit_signal_is_not_retried() {
        // A genuine permission error (private repo, bad token scope, ...)
        // must not be retried — retrying it wastes time and looks like
        // hammering a server that has already said no.
        let empty = HeaderMap::new();
        assert!(!should_retry(StatusCode::FORBIDDEN, &empty, 1, 3));
    }

    #[test]
    fn forbidden_is_retried_when_it_carries_a_rate_limit_signal() {
        let mut with_retry_after = HeaderMap::new();
        with_retry_after.insert(
            HeaderName::from_static("retry-after"),
            HeaderValue::from_static("5"),
        );
        assert!(should_retry(StatusCode::FORBIDDEN, &with_retry_after, 1, 3));

        let mut exhausted = HeaderMap::new();
        exhausted.insert(
            HeaderName::from_static("x-ratelimit-remaining"),
            HeaderValue::from_static("0"),
        );
        assert!(should_retry(StatusCode::FORBIDDEN, &exhausted, 1, 3));
    }

    #[test]
    fn retries_stop_once_max_attempts_reached() {
        let empty = HeaderMap::new();
        assert!(!should_retry(StatusCode::TOO_MANY_REQUESTS, &empty, 3, 3));
        assert!(!should_retry(StatusCode::BAD_GATEWAY, &empty, 5, 3));
    }

    #[test]
    fn backoff_delay_grows_with_attempt_number() {
        assert!(backoff_delay(3) >= backoff_delay(1));
        assert!(backoff_delay(6) >= backoff_delay(3));
    }

    #[test]
    fn backoff_delay_growth_is_capped_so_it_cannot_overflow() {
        // attempt is saturating-capped internally; this just proves a
        // large attempt number doesn't panic or produce a degenerate value.
        let d = backoff_delay(1000);
        assert!(d.as_secs() < 3600);
    }

    #[test]
    fn retry_after_header_is_honored_over_backoff() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("retry-after"),
            HeaderValue::from_static("42"),
        );
        assert_eq!(retry_delay(&headers, 1), Duration::from_secs(42));
    }

    #[test]
    fn proactive_wait_is_none_when_headers_absent_or_remaining_healthy() {
        assert_eq!(proactive_wait_secs(None, None, 2, 900), None);
        assert_eq!(
            proactive_wait_secs(Some(500), Some(1_900_000_000), 2, 900),
            None
        );
    }

    #[test]
    fn proactive_wait_is_capped_at_max_wait_secs() {
        let far_future = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 10_000;
        let wait = proactive_wait_secs(Some(0), Some(far_future), 2, 900).unwrap();
        assert!(wait <= 900);
    }

    #[test]
    fn proactive_wait_is_none_when_reset_already_passed() {
        let past = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - 10;
        assert_eq!(proactive_wait_secs(Some(0), Some(past), 2, 900), None);
    }
}
