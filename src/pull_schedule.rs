//! Pure scheduled-pull policy.
//!
//! Keeping this separate from Wi-Fi lets the battery/network timing policy be
//! tested without an ESP-IDF runtime. A human-requested pull always bypasses
//! a failure backoff; only autonomous retries are delayed.

pub const INITIAL_RETRY_MS: u64 = 30_000;
pub const MAX_RETRY_MS: u64 = 60 * 60 * 1_000;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PullSchedule {
    next_due_ms: u64,
    consecutive_failures: u8,
}

impl PullSchedule {
    pub fn restore(next_due_ms: u64, consecutive_failures: u8) -> Self {
        Self {
            next_due_ms,
            consecutive_failures,
        }
    }

    pub fn is_due(&self, now_ms: u64, manually_requested: bool) -> bool {
        manually_requested || now_ms >= self.next_due_ms
    }

    pub fn succeeded(&mut self, now_ms: u64, interval_seconds: u64) {
        self.consecutive_failures = 0;
        self.next_due_ms = now_ms.saturating_add(interval_seconds.saturating_mul(1_000));
    }

    pub fn failed(&mut self, now_ms: u64) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        let shift = u32::from(self.consecutive_failures.saturating_sub(1)).min(16);
        let delay = INITIAL_RETRY_MS
            .saturating_mul(1_u64 << shift)
            .min(MAX_RETRY_MS);
        self.next_due_ms = now_ms.saturating_add(delay);
    }

    pub fn next_due_ms(&self) -> u64 {
        self.next_due_ms
    }

    pub fn consecutive_failures(&self) -> u8 {
        self.consecutive_failures
    }

    /// Round up so a subsecond remainder cannot cause an early pull.
    pub fn seconds_until_due(&self, now_ms: u64) -> u64 {
        self.next_due_ms
            .saturating_sub(now_ms)
            .div_ceil(1_000)
            .max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_attempt_is_due_immediately() {
        assert!(PullSchedule::default().is_due(0, false));
    }

    #[test]
    fn failures_back_off_exponentially_and_cap() {
        let mut schedule = PullSchedule::default();
        schedule.failed(1_000);
        assert_eq!(schedule.next_due_ms(), 31_000);
        schedule.failed(31_000);
        assert_eq!(schedule.next_due_ms(), 91_000);
        for _ in 0..20 {
            let now = schedule.next_due_ms();
            schedule.failed(now);
        }
        let before_final_failure = schedule.next_due_ms();
        schedule.failed(before_final_failure);
        assert_eq!(schedule.next_due_ms() - before_final_failure, MAX_RETRY_MS);
    }

    #[test]
    fn manual_request_bypasses_backoff() {
        let mut schedule = PullSchedule::default();
        schedule.failed(0);
        assert!(!schedule.is_due(1, false));
        assert!(schedule.is_due(1, true));
    }

    #[test]
    fn minute_wakes_do_not_advance_a_fifteen_minute_pull() {
        let mut schedule = PullSchedule::default();
        schedule.succeeded(0, 900);
        for minute in 1..15 {
            // Emulate reconstructing the scheduler on every deep-sleep boot.
            schedule =
                PullSchedule::restore(schedule.next_due_ms(), schedule.consecutive_failures());
            assert!(!schedule.is_due(minute * 60_000, false));
        }
        assert!(schedule.is_due(900_000, false));
        assert_eq!(schedule.seconds_until_due(899_001), 1);
    }

    #[test]
    fn failure_backoff_survives_clock_only_wakes() {
        let mut schedule = PullSchedule::default();
        for _ in 0..4 {
            let now = schedule.next_due_ms();
            schedule.failed(now);
        }
        let restored =
            PullSchedule::restore(schedule.next_due_ms(), schedule.consecutive_failures());
        assert_eq!(restored, schedule);
        assert!(!restored.is_due(schedule.next_due_ms() - 1, false));
    }
}
