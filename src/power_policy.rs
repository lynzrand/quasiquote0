//! Recovery-first decision shared by the clock and frame client.

pub const COLD_BOOT_RECOVERY_MS: u64 = 60_000;
pub const TIMER_WAKE_RECOVERY_MS: u64 = 2_000;

#[derive(Clone, Copy, Debug)]
pub struct AwakeGuard {
    pub vbus_present: bool,
    pub usb_host_connected: bool,
    pub uptime_ms: u64,
    pub timer_wake: bool,
}

impl AwakeGuard {
    pub fn must_stay_awake(self) -> bool {
        let recovery_ms = if self.timer_wake {
            TIMER_WAKE_RECOVERY_MS
        } else {
            COLD_BOOT_RECOVERY_MS
        };
        self.vbus_present || self.usb_host_connected || self.uptime_ms < recovery_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn battery_boot(uptime_ms: u64, timer_wake: bool) -> AwakeGuard {
        AwakeGuard {
            vbus_present: false,
            usb_host_connected: false,
            uptime_ms,
            timer_wake,
        }
    }

    #[test]
    fn usb_power_or_host_keeps_device_awake_after_recovery_expires() {
        let idle = battery_boot(24 * 60 * 60 * 1_000, true);
        assert!(!idle.must_stay_awake());
        assert!(AwakeGuard {
            vbus_present: true,
            ..idle
        }
        .must_stay_awake());
        assert!(AwakeGuard {
            usb_host_connected: true,
            ..idle
        }
        .must_stay_awake());
    }

    #[test]
    fn cold_boot_allows_a_full_minute_to_recover_even_with_bad_vbus_sensing() {
        assert!(battery_boot(COLD_BOOT_RECOVERY_MS - 1, false).must_stay_awake());
        assert!(!battery_boot(COLD_BOOT_RECOVERY_MS, false).must_stay_awake());
    }

    #[test]
    fn timer_wake_allows_usb_enumeration_before_sleeping_again() {
        assert!(battery_boot(TIMER_WAKE_RECOVERY_MS - 1, true).must_stay_awake());
        assert!(!battery_boot(TIMER_WAKE_RECOVERY_MS, true).must_stay_awake());
    }
}
