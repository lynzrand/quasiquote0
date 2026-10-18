//! Shared device state for the browser, USB bootstrap, and main owner task.
use crate::{activity::OPERATIONS, config::Config, device::Device};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc, Mutex,
};

#[derive(Clone, Default, Serialize)]
pub struct NetworkStatus {
    pub mode: &'static str,
    pub connected: bool,
    pub ip: Option<String>,
    pub url: Option<String>,
    pub error: Option<String>,
    pub last_ntp_unix_seconds: Option<i64>,
}

pub struct App {
    pub device: Option<Arc<Mutex<Device>>>,
    pub recovery_error: Option<String>,
    pub network: Mutex<NetworkStatus>,
    pub pull_requested: AtomicBool,
    pub reboot_requested: AtomicBool,
    pub main_stack_free: AtomicU32,
    pub bluetooth_advertising: AtomicBool,
    pub wifi_change_after_ms: Mutex<u64>,
    pub setup_ssid: String,
    pub access_token: String,
}

impl App {
    pub fn mount() -> Arc<Self> {
        let (device, mut error) = match Device::mount() {
            Ok(device) => (Some(Arc::new(Mutex::new(device))), None),
            Err(error) => (None, Some(error.to_string())),
        };
        let generated = || {
            (0..4)
                .map(|_| format!("{:08x}", unsafe { esp_idf_sys::esp_random() }))
                .collect::<String>()
        };
        let access_token = if let Some(device) = &device {
            let device = device.lock().unwrap();
            match device.admin_token() {
                Ok(Some(token))
                    if token.len() == 32 && token.bytes().all(|b| b.is_ascii_hexdigit()) =>
                {
                    token
                }
                _ => {
                    let token = generated();
                    if let Err(e) = device.save_admin_token(&token) {
                        error = Some(e.to_string());
                    }
                    token
                }
            }
        } else {
            generated()
        };
        let mut mac = [0u8; 6];
        unsafe {
            esp_idf_sys::esp_efuse_mac_get_default(mac.as_mut_ptr());
        }
        Arc::new(Self {
            device,
            recovery_error: error,
            network: Mutex::new(NetworkStatus {
                mode: "off",
                ..Default::default()
            }),
            pull_requested: AtomicBool::new(false),
            reboot_requested: AtomicBool::new(false),
            main_stack_free: AtomicU32::new(0),
            bluetooth_advertising: AtomicBool::new(false),
            wifi_change_after_ms: Mutex::new(0),
            setup_ssid: format!("Quote0-{:02X}{:02X}{:02X}", mac[3], mac[4], mac[5]),
            access_token,
        })
    }
    pub fn usb_powered(&self) -> bool {
        crate::battery::vbus_present() || unsafe { esp_idf_sys::usb_serial_jtag_is_connected() }
    }
    pub fn authorized(&self, token: Option<&str>) -> bool {
        token.is_some_and(|token| {
            token.len() == self.access_token.len()
                && token
                    .bytes()
                    .zip(self.access_token.bytes())
                    .fold(0u8, |a, (x, y)| a | (x ^ y))
                    == 0
        })
    }
    pub fn config(&self) -> Config {
        self.device
            .as_ref()
            .map(|d| d.lock().unwrap().config().clone())
            .unwrap_or_default()
    }
    pub fn replace_config(&self, mut config: Config) -> Result<(), String> {
        let _operation = OPERATIONS
            .try_acquire()
            .ok_or("Another operation is in progress")?;
        let mut device = self
            .device
            .as_ref()
            .ok_or("Storage recovery required")?
            .lock()
            .unwrap();
        config.preserve_secrets(device.config());
        // Leave time for the browser's save response before changing its link.
        *self.wifi_change_after_ms.lock().unwrap() =
            crate::rtc_schedule::now_ms().saturating_add(1_000);
        device.replace_config(config).map_err(|e| e.to_string())?;
        self.pull_requested
            .store(device.config().pull.enabled, Ordering::Release);
        Ok(())
    }
    pub fn set_wifi(&self, ssid: String, password: String) -> Result<(), String> {
        // Acquire before reading the previous configuration to avoid lost edits.
        let _operation = OPERATIONS
            .try_acquire()
            .ok_or("Another operation is in progress")?;
        let mut device = self
            .device
            .as_ref()
            .ok_or("Storage recovery required")?
            .lock()
            .unwrap();
        let mut config = device.config().clone();
        config.wifi.ssid = ssid;
        config.wifi.password = password;
        if config.wifi.ssid.is_empty() {
            config.pull.enabled = false;
        }
        *self.wifi_change_after_ms.lock().unwrap() =
            crate::rtc_schedule::now_ms().saturating_add(1_000);
        device.replace_config(config).map_err(|e| e.to_string())
    }
    pub fn status(&self, physical_usb: bool) -> Value {
        let guard = crate::power::guard();
        let (armed, wake_error) = crate::power::wake_status();
        let battery = crate::battery::read();
        let mut status = json!({"firmware":env!("CARGO_PKG_VERSION"),"mode":"browser","device":self.setup_ssid,
            "network":self.network.lock().unwrap().clone(),"clock":crate::wall_clock::status(),
            "power":{"usb_present":battery.usb_present,"must_stay_awake":guard.must_stay_awake(),
                "uptime_ms":guard.uptime_ms,"timer_wake":guard.timer_wake,"wake_sources_armed":armed,
                "wake_error":wake_error,"reset_reason":unsafe{esp_idf_sys::esp_reset_reason()},
                "wake_cause":unsafe{esp_idf_sys::esp_sleep_get_wakeup_cause()}},
            "battery":{"millivolts":battery.millivolts,"percent":battery.percent},
            "memory":{"free_bytes":unsafe{esp_idf_sys::esp_get_free_heap_size()},
                "main_stack_min_free_bytes":self.main_stack_free.load(Ordering::Relaxed),
                "minimum_free_bytes":unsafe{esp_idf_sys::esp_get_minimum_free_heap_size()},
                "largest_block_bytes":unsafe{esp_idf_sys::heap_caps_get_largest_free_block(esp_idf_sys::MALLOC_CAP_8BIT)}},
            "bluetooth":{"advertising":self.bluetooth_advertising.load(Ordering::Relaxed)},
            "operation_active":OPERATIONS.is_active(),"recovery_error":self.recovery_error});
        if let Some(device) = &self.device {
            let device = device.lock().unwrap();
            let state = device.state();
            status["ota"] = json!({"running_partition":crate::ota::running_partition_label(),"pending":state.ota_pending});
            status["configured"] = json!(!device.config().wifi.ssid.is_empty());
            status["display"] = json!({"clock_unix_seconds":device.display_clock,"refreshes":device.display_refreshes,
                "time":device.display_clock.map(|s|crate::clock_face::ClockFace::at(s).text()),
                "refresh_mode":device.display_refresh_mode,"refresh_ms":device.display_refresh_ms,
                "error":device.display_error,"partial_error":device.partial_refresh_error,
                "desired_sha256":state.desired.as_ref().map(|d|&d.sha256),
                "displayed_content_sha256":state.displayed_content_sha256,"composed_sha256":state.displayed_composed_sha256});
            status["pull"] = json!({"enabled":device.config().pull.enabled,"attempts":device.pull_attempts,
                "interval_seconds":device.config().pull.interval_seconds,"last_error":device.pull_error()});
            if let Ok((total, used)) = device.filesystem_info() {
                status["filesystem"] = json!({"total_bytes":total,"used_bytes":used});
            }
        }
        if physical_usb {
            status["access_token"] = json!(self.access_token);
            status["setup"] = json!({"ssid":self.setup_ssid,"password":&self.access_token[..12]});
        }
        status
    }
}
