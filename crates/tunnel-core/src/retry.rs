use std::time::Duration;
use tunnel_domain::RetryPolicy;

/// Pure retry timing. The supervisor supplies random jitter and owns the
/// cancellation-aware wait; this type never sleeps or starts a task.
pub struct RetrySchedule {
    policy: RetryPolicy,
}

impl RetrySchedule {
    pub fn new(policy: RetryPolicy) -> Self {
        Self { policy }
    }

    /// `attempt` starts at 1. `jitter_percent` is an external random value;
    /// values outside the configured range are clamped.
    pub fn delay(&self, attempt: u32, jitter_percent: i8) -> Duration {
        const MULTIPLIERS: [u64; 6] = [1, 2, 5, 10, 20, 30];
        let index = attempt.saturating_sub(1).min(5) as usize;
        let base_ms = self.policy.base_delay.as_millis();
        let max_ms = self.policy.max_delay.as_millis();
        let unjittered_ms = base_ms
            .saturating_mul(MULTIPLIERS[index] as u128)
            .min(max_ms);
        let limit = i16::from(self.policy.jitter_percent.min(100));
        let jitter = i16::from(jitter_percent).clamp(-limit, limit);
        let adjusted_ms = unjittered_ms.saturating_mul((100 + jitter) as u128) / 100;
        Duration::from_millis(adjusted_ms.min(u64::MAX as u128) as u64)
    }

    pub fn should_reset(&self, healthy_for: Duration) -> bool {
        healthy_for >= self.policy.reset_after_healthy
    }

    pub fn jitter_limit(&self) -> i8 {
        self.policy.jitter_percent.min(100) as i8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_sequence_caps_at_thirty_seconds() {
        let schedule = RetrySchedule::new(RetryPolicy::default());
        let seconds: Vec<_> = (1..=8)
            .map(|attempt| schedule.delay(attempt, 0).as_secs())
            .collect();
        assert_eq!(seconds, [1, 2, 5, 10, 20, 30, 30, 30]);
    }

    #[test]
    fn jitter_is_bounded_and_short_connections_do_not_reset() {
        let schedule = RetrySchedule::new(RetryPolicy::default());
        assert_eq!(schedule.delay(3, -50), Duration::from_secs(4));
        assert_eq!(schedule.delay(3, 50), Duration::from_secs(6));
        assert!(!schedule.should_reset(Duration::from_secs(2)));
        assert!(schedule.should_reset(Duration::from_secs(60)));
    }
}
