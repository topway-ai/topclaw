use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Lock-free circuit breaker for provider failover.
///
/// Tracks consecutive failures per provider and temporarily skips providers
/// that have exceeded the failure threshold, avoiding wasted latency on
/// providers that are consistently failing.
///
/// States:
/// - **Closed** (healthy): failures < threshold — provider is attempted normally.
/// - **Open** (tripped): failures >= threshold AND cooldown not expired — provider is skipped.
/// - **Half-open**: failures >= threshold BUT cooldown expired — one probe attempt is allowed.
///
/// A threshold of 0 disables the circuit breaker (never opens).
pub struct CircuitBreaker {
    consecutive_failures: AtomicU32,
    last_failure_epoch_ms: AtomicU64,
    threshold: u32,
    cooldown_ms: u64,
}

impl CircuitBreaker {
    /// Create a new circuit breaker.
    ///
    /// - `threshold`: number of consecutive failures before the circuit opens.
    ///   A value of 0 disables the circuit breaker entirely.
    /// - `cooldown_ms`: milliseconds after the last failure before the circuit
    ///   transitions to half-open and allows a probe attempt.
    pub fn new(threshold: u32, cooldown_ms: u64) -> Self {
        Self {
            consecutive_failures: AtomicU32::new(0),
            last_failure_epoch_ms: AtomicU64::new(0),
            threshold,
            cooldown_ms,
        }
    }

    /// Returns `true` if the circuit is open (provider should be skipped).
    ///
    /// When the cooldown has expired the circuit enters half-open state and
    /// this returns `false` to allow a single probe attempt.
    pub fn is_open(&self) -> bool {
        if self.threshold == 0 {
            return false;
        }

        let failures = self.consecutive_failures.load(Ordering::Relaxed);
        if failures < self.threshold {
            return false;
        }

        let last_failure = self.last_failure_epoch_ms.load(Ordering::Relaxed);
        let now = now_epoch_ms();

        // Cooldown expired → half-open: allow one probe attempt.
        if now.saturating_sub(last_failure) >= self.cooldown_ms {
            return false;
        }

        true
    }

    /// Record a successful provider call, resetting the failure counter.
    pub fn record_success(&self) {
        self.consecutive_failures.store(0, Ordering::Relaxed);
    }

    /// Record a failed provider call, incrementing the failure counter
    /// and updating the last-failure timestamp.
    pub fn record_failure(&self) {
        self.consecutive_failures.fetch_add(1, Ordering::Relaxed);
        self.last_failure_epoch_ms
            .store(now_epoch_ms(), Ordering::Relaxed);
    }
}

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_when_threshold_zero() {
        let cb = CircuitBreaker::new(0, 1000);
        for _ in 0..100 {
            cb.record_failure();
        }
        assert!(!cb.is_open(), "threshold=0 should never open the circuit");
    }

    #[test]
    fn stays_closed_below_threshold() {
        let cb = CircuitBreaker::new(3, 5000);
        cb.record_failure();
        cb.record_failure();
        assert!(
            !cb.is_open(),
            "2 failures should not open a threshold-3 breaker"
        );
    }

    #[test]
    fn opens_at_threshold() {
        let cb = CircuitBreaker::new(3, 60_000);
        cb.record_failure();
        cb.record_failure();
        cb.record_failure();
        assert!(cb.is_open(), "3 failures should open a threshold-3 breaker");
    }

    #[test]
    fn success_resets_failures() {
        let cb = CircuitBreaker::new(3, 60_000);
        cb.record_failure();
        cb.record_failure();
        cb.record_success();
        cb.record_failure();
        assert!(
            !cb.is_open(),
            "success should reset counter so 1 failure < threshold"
        );
    }

    #[test]
    fn half_open_after_cooldown_expires() {
        let cb = CircuitBreaker::new(2, 50);
        cb.record_failure();
        cb.record_failure();
        assert!(cb.is_open(), "should be open immediately after threshold");

        // Wait for cooldown to expire
        std::thread::sleep(std::time::Duration::from_millis(60));
        assert!(
            !cb.is_open(),
            "should be half-open (closed) after cooldown expires"
        );
    }

    #[test]
    fn re_opens_after_half_open_failure() {
        let cb = CircuitBreaker::new(2, 50);
        cb.record_failure();
        cb.record_failure();

        // Wait for cooldown
        std::thread::sleep(std::time::Duration::from_millis(60));
        assert!(!cb.is_open(), "half-open should allow probe");

        // Probe fails → circuit re-opens with fresh timestamp
        cb.record_failure();
        assert!(
            cb.is_open(),
            "should re-open after failure in half-open state"
        );
    }
}
