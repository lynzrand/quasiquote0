//! Content changes and minute heartbeats share one bounded refresh path.

use crate::frame::Sha256Digest;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RefreshKey {
    pub content: Sha256Digest,
    pub status_bar: bool,
    pub minute: Option<i64>,
}

impl RefreshKey {
    pub fn new(content: Sha256Digest, status_bar: bool, seconds: i64) -> Self {
        Self {
            content,
            status_bar,
            minute: status_bar.then_some(seconds.div_euclid(60)),
        }
    }
}

#[derive(Default)]
pub struct RefreshTracker {
    displayed: Option<RefreshKey>,
    retry_after_ms: u64,
}

impl RefreshTracker {
    pub fn is_due(&self, key: RefreshKey, now_ms: u64) -> bool {
        self.displayed != Some(key) && now_ms >= self.retry_after_ms
    }

    pub fn succeeded(&mut self, key: RefreshKey) {
        self.displayed = Some(key);
        self.retry_after_ms = 0;
    }

    pub fn failed(&mut self, now_ms: u64) {
        self.retry_after_ms = now_ms.saturating_add(5_000);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minute_heartbeat_and_content_change_each_schedule_a_refresh() {
        let mut tracker = RefreshTracker::default();
        let first = RefreshKey::new([1; 32], true, 120);
        assert!(tracker.is_due(first, 0));
        tracker.succeeded(first);
        assert!(!tracker.is_due(RefreshKey::new([1; 32], true, 179), 1_000));
        assert!(tracker.is_due(RefreshKey::new([1; 32], true, 180), 2_000));
        assert!(tracker.is_due(RefreshKey::new([2; 32], true, 120), 2_000));
    }

    #[test]
    fn disabling_the_bar_disables_heartbeat_but_refreshes_the_layout() {
        let mut tracker = RefreshTracker::default();
        tracker.succeeded(RefreshKey::new([1; 32], true, 120));
        let disabled = RefreshKey::new([1; 32], false, 120);
        assert!(tracker.is_due(disabled, 0));
        tracker.succeeded(disabled);
        assert!(!tracker.is_due(RefreshKey::new([1; 32], false, 3_600), 1));
    }

    #[test]
    fn failed_refresh_is_retried_with_backoff_and_never_acknowledged() {
        let mut tracker = RefreshTracker::default();
        let first = RefreshKey::new([1; 32], true, 120);
        tracker.failed(100);
        assert!(!tracker.is_due(first, 5_099));
        assert!(tracker.is_due(first, 5_100));
        tracker.succeeded(first);
        assert!(!tracker.is_due(first, 5_100));
    }
}
