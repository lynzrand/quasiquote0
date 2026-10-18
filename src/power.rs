//! Battery-mode power policy: deep sleep, wake sources, and thresholds.
//!
//! While VBUS is present and `power.stay_awake_on_usb` is set, the device
//! stays awake and serves USB. On battery it performs one conditional pull
//! per wake and returns to deep sleep. A critically low battery skips the
//! radio and panel entirely and sleeps until VBUS or a safety timer.

/// Below this terminal voltage the device stops refreshing the panel and
/// stops transmitting so the cell survives until charging.
pub const CRITICAL_BATTERY_MV: u32 = 3_300;

/// Safety wake interval at critical battery: long, but bounded so a device
/// left on a dead cell still recovers when charged.
pub const CRITICAL_SLEEP_SECONDS: u64 = 6 * 60 * 60;

/// Wake interval when there is nothing scheduled to do (pull disabled or
/// unconfigured) and the device is on battery: reachable-in-principle
/// without cycling the radio.
pub const IDLE_SLEEP_SECONDS: u64 = 60 * 60;

/// Wake interval when the device sleeps *despite* VBUS being present
/// (`stay_awake_on_usb: false`), or when a critical battery is charging.
/// The ESP32-C3 cannot wake on a level that is already asserted, so VBUS
/// wake cannot be armed while USB is attached; a short timer keeps the
/// device recoverable through configuration changes instead, and VBUS power
/// makes the frequent wakes effectively free.
pub const USB_ATTACHED_SLEEP_SECONDS: u64 = 60;

/// Enter deep sleep for `seconds`, optionally waking when VBUS appears.
///
/// GPIO0 (VBUS sense) is RTC-capable on the ESP32-C3, so a high level there
/// reboots the device into USB-powered mode. The internal resistor matching
/// the wakeup level is applied by ESP-IDF automatically. Deep sleep never
/// returns; the next boot is a cold boot and state comes from LittleFS.
pub fn enter_deep_sleep(seconds: u64, wake_on_vbus: bool) -> ! {
    unsafe {
        let _ = esp_idf_sys::esp_sleep_enable_timer_wakeup(seconds * 1_000_000);
        if wake_on_vbus && esp_idf_sys::esp_sleep_is_valid_wakeup_gpio(0) {
            let _ = esp_idf_sys::esp_deep_sleep_enable_gpio_wakeup(
                1 << 0,
                esp_idf_sys::esp_deepsleep_gpio_wake_up_mode_t_ESP_GPIO_WAKEUP_GPIO_HIGH,
            );
        }
        esp_idf_sys::esp_deep_sleep_start();
    }
}
