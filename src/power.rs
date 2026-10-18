//! USB always wins over configuration and battery sleep.

use crate::power_policy::AwakeGuard;
use std::sync::Mutex;

static WAKE_RESULT: Mutex<Option<Result<(), &'static str>>> = Mutex::new(None);

pub fn wake_status() -> (bool, Option<&'static str>) {
    match *WAKE_RESULT.lock().unwrap() {
        Some(Ok(())) => (true, None),
        Some(Err(error)) => (false, Some(error)),
        None => (false, None),
    }
}

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

pub fn guard() -> AwakeGuard {
    AwakeGuard {
        vbus_present: crate::battery::vbus_present(),
        usb_host_connected: unsafe { esp_idf_sys::usb_serial_jtag_is_connected() },
        uptime_ms: (unsafe { esp_idf_sys::esp_timer_get_time() } / 1_000) as u64,
        timer_wake: unsafe {
            esp_idf_sys::esp_sleep_get_wakeup_cause()
                == esp_idf_sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_TIMER
        },
    }
}

pub fn must_stay_awake() -> bool {
    guard().must_stay_awake()
}

/// Both wake sources are mandatory. On any error the caller stays awake.
pub fn arm_wake_sources(seconds: u64) -> Result<(), &'static str> {
    let result = arm_wake_sources_inner(seconds);
    *WAKE_RESULT.lock().unwrap() = Some(result);
    result
}

fn arm_wake_sources_inner(seconds: u64) -> Result<(), &'static str> {
    let micros = seconds
        .checked_mul(1_000_000)
        .filter(|v| *v > 0)
        .ok_or("invalid sleep interval")?;
    unsafe {
        if !esp_idf_sys::esp_sleep_is_valid_wakeup_gpio(0) {
            return Err("GPIO0 cannot wake this chip");
        }
        if esp_idf_sys::esp_sleep_enable_timer_wakeup(micros) != esp_idf_sys::ESP_OK {
            return Err("timer wake could not be armed");
        }
        if esp_idf_sys::esp_deep_sleep_enable_gpio_wakeup(
            1,
            esp_idf_sys::esp_deepsleep_gpio_wake_up_mode_t_ESP_GPIO_WAKEUP_GPIO_HIGH,
        ) != esp_idf_sys::ESP_OK
        {
            return Err("USB insertion wake could not be armed");
        }
    }
    Ok(())
}

/// Returns if USB, recovery time, or an upload prevents sleeping. GPIO0 is
/// armed before the final check, so insertion during that race wakes us again.
pub fn try_deep_sleep(seconds: u64) -> Result<(), &'static str> {
    if must_stay_awake() || crate::rpc::uploads_active() {
        return Ok(());
    }
    arm_wake_sources(seconds)?;
    if must_stay_awake() || crate::rpc::uploads_active() {
        return Ok(());
    }
    unsafe { esp_idf_sys::esp_deep_sleep_start() }
}
