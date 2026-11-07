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
    content_updates: u8,
    // Acknowledging identical pixels does not establish a controller baseline.
    has_baseline: bool,
}

impl RefreshTracker {
    pub fn is_due(&self, key: RefreshKey, now_ms: u64) -> bool {
        self.displayed != Some(key) && now_ms >= self.retry_after_ms
    }

    pub fn succeeded(&mut self, key: RefreshKey) {
        self.displayed = Some(key);
        self.retry_after_ms = 0;
        self.content_updates = 0;
        self.has_baseline = true;
    }

    pub fn unchanged(&mut self, key: RefreshKey) {
        self.displayed = Some(key);
        self.retry_after_ms = 0;
    }

    pub fn can_refresh_status_only(&self, key: RefreshKey) -> bool {
        self.has_baseline
            && self.displayed.is_some_and(|previous| {
                previous.status_bar && key.status_bar && previous.content == key.content
            })
    }

    pub fn can_refresh_differential(&self) -> bool {
        self.has_baseline && self.content_updates < 4 && self.displayed.is_some()
    }

    pub fn partial_succeeded(&mut self, key: RefreshKey) {
        // Clock-only windows leave the main image untouched. They must not
        // turn an idle display into a periodic full-screen refresh.
        if self.displayed.is_some_and(|previous| {
            previous.content != key.content || previous.status_bar != key.status_bar
        }) {
            self.content_updates += 1;
        }
        self.displayed = Some(key);
        self.retry_after_ms = 0;
    }

    pub fn failed(&mut self, now_ms: u64) {
        // A failed transfer may have altered visible pixels. The next
        // attempt must establish a full baseline before another window.
        self.displayed = None;
        self.content_updates = 0;
        self.has_baseline = false;
        self.retry_after_ms = now_ms.saturating_add(5_000);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_pixels_neither_create_a_baseline_nor_reset_the_cleanup_budget() {
        let mut tracker = RefreshTracker::default();
        let key = RefreshKey::new([1; 32], true, 0);
        tracker.unchanged(key);
        assert!(!tracker.is_due(key, 0));
        assert!(!tracker.can_refresh_differential());
        assert!(!tracker.can_refresh_status_only(key));
        tracker.succeeded(key);
        for minute in 1..=4 {
            tracker.partial_succeeded(RefreshKey::new([minute as u8 + 1; 32], true, minute * 60));
        }
        let unchanged = RefreshKey::new([2; 32], true, 300);
        tracker.unchanged(unchanged);
        assert!(!tracker.is_due(unchanged, 0));
        assert!(!tracker.can_refresh_differential());
        assert!(tracker.can_refresh_status_only(unchanged));
    }

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
        tracker.failed(5_200);
        assert!(!tracker.can_refresh_status_only(first));
        assert!(tracker.is_due(first, 10_200));
    }

    #[test]
    fn status_windows_require_a_full_baseline_but_never_schedule_idle_cleanup() {
        let mut tracker = RefreshTracker::default();
        let first = RefreshKey::new([1; 32], true, 0);
        assert!(!tracker.can_refresh_status_only(first));
        tracker.succeeded(first);
        assert!(!tracker.can_refresh_status_only(RefreshKey::new([2; 32], true, 60)));
        assert!(!tracker.can_refresh_status_only(RefreshKey::new([1; 32], false, 60)));
        for minute in 1..=20 {
            let key = RefreshKey::new([1; 32], true, minute * 60);
            assert!(tracker.can_refresh_status_only(key));
            tracker.partial_succeeded(key);
        }
        assert!(tracker.can_refresh_differential());
    }

    #[test]
    fn main_image_changes_still_require_periodic_full_baselines() {
        let mut tracker = RefreshTracker::default();
        tracker.succeeded(RefreshKey::new([1; 32], true, 0));
        for content in 2..=5 {
            assert!(tracker.can_refresh_differential());
            tracker.partial_succeeded(RefreshKey::new([content; 32], true, 0));
        }
        let clock = RefreshKey::new([5; 32], true, 60);
        assert!(tracker.can_refresh_status_only(clock));
        tracker.partial_succeeded(clock);
        assert!(!tracker.can_refresh_differential());
        let cleanup = RefreshKey::new([6; 32], true, 60);
        tracker.succeeded(cleanup);
        assert!(tracker.can_refresh_differential());
    }
}
