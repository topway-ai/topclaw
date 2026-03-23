//! Background health probes for provider circuit breakers.
//!
//! When enabled, a background tokio task periodically calls `warmup()` on each
//! provider. Success resets the circuit breaker; failure increments it. This
//! pre-warms the circuit breaker state so the first user request after an
//! outage doesn't eat the full failover latency.

use super::circuit_breaker::CircuitBreaker;
use super::Provider;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

/// Spawn a background health probe task.
///
/// The task calls `warmup()` on each provider every `interval_secs` seconds,
/// updating the corresponding circuit breaker on success or failure.
///
/// Returns a `JoinHandle` that should be stored by the caller to keep the task
/// alive. Dropping the handle aborts the task on next `.await`.
pub fn spawn_health_probes(
    providers: Vec<(String, Arc<dyn Provider>)>,
    circuit_breakers: Vec<Arc<CircuitBreaker>>,
    interval_secs: u64,
) -> JoinHandle<()> {
    let interval = Duration::from_secs(interval_secs.max(5));

    tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval);
        // Skip the first immediate tick — providers were just created.
        tick.tick().await;

        loop {
            tick.tick().await;

            for (i, (name, provider)) in providers.iter().enumerate() {
                if let Some(cb) = circuit_breakers.get(i) {
                    match provider.warmup().await {
                        Ok(()) => {
                            cb.record_success();
                            tracing::trace!(provider = name.as_str(), "Health probe succeeded");
                        }
                        Err(e) => {
                            cb.record_failure();
                            tracing::debug!(
                                provider = name.as_str(),
                                error = %e,
                                "Health probe failed"
                            );
                        }
                    }
                }
            }
        }
    })
}
