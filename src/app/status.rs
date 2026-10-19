//! Management snapshots; physical USB alone exposes setup credentials.
use super::App;
use crate::{activity::OPERATIONS, clock_face::ClockFace, device::Device};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;

impl App {
    pub fn status(&self, physical_usb: bool) -> Value {
        let guard = crate::power::guard();
        let (armed, wake_error) = crate::power::wake_status();
        let battery = crate::battery::read();
        let mut status = json!({
            "firmware": env!("CARGO_PKG_VERSION"),
            "mode": "browser",
            "device": self.setup_ssid,
            "network": self.network.lock().unwrap().clone(),
            "clock": crate::wall_clock::status(),
            "power": {
                "usb_present": battery.usb_present,
                "must_stay_awake": guard.must_stay_awake(),
                "uptime_ms": guard.uptime_ms,
                "timer_wake": guard.timer_wake,
                "wake_sources_armed": armed,
                "wake_error": wake_error,
                "reset_reason": unsafe { esp_idf_sys::esp_reset_reason() },
                "wake_cause": unsafe { esp_idf_sys::esp_sleep_get_wakeup_cause() },
            },
            "battery": { "millivolts": battery.millivolts, "percent": battery.percent },
            "memory": {
                "free_bytes": unsafe { esp_idf_sys::esp_get_free_heap_size() },
                "main_stack_min_free_bytes": self.main_stack_free.load(Ordering::Relaxed),
                "minimum_free_bytes": unsafe { esp_idf_sys::esp_get_minimum_free_heap_size() },
                "largest_block_bytes": unsafe {
                    esp_idf_sys::heap_caps_get_largest_free_block(esp_idf_sys::MALLOC_CAP_8BIT)
                },
            },
            "bluetooth": { "advertising": self.bluetooth_advertising.load(Ordering::Relaxed) },
            "operation_active": OPERATIONS.is_active(),
            "recovery_error": self.recovery_error,
        });
        if let Some(device) = &self.device {
            add_device_status(&mut status, &device.lock().unwrap());
        }
        if physical_usb {
            status["access_token"] = json!(self.access_token);
            status["setup"] =
                json!({ "ssid": self.setup_ssid, "password": &self.access_token[..12] });
        }
        status
    }
}

fn add_device_status(status: &mut Value, device: &Device) {
    let state = device.state();
    status["ota"] = json!({
        "running_partition": crate::ota::running_partition_label(),
        "pending": state.ota_pending,
    });
    status["configured"] = json!(!device.config().wifi.ssid.is_empty());
    status["display"] = json!({
        "clock_unix_seconds": device.display_clock,
        "refreshes": device.display_refreshes,
        "time": device.display_clock.map(|seconds| ClockFace::at(seconds).text()),
        "refresh_mode": device.display_refresh_mode,
        "refresh_ms": device.display_refresh_ms,
        "error": device.display_error,
        "partial_error": device.partial_refresh_error,
        "desired_sha256": state.desired.as_ref().map(|desired| &desired.sha256),
        "displayed_content_sha256": state.displayed_content_sha256,
        "composed_sha256": state.displayed_composed_sha256,
    });
    status["pull"] = json!({
        "enabled": device.config().pull.enabled,
        "attempts": device.pull_attempts,
        "interval_seconds": device.config().pull.interval_seconds,
        "last_error": device.pull_error(),
    });
    if let Ok((total, used)) = device.filesystem_info() {
        status["filesystem"] = json!({ "total_bytes": total, "used_bytes": used });
    }
}
