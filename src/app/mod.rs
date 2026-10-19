//! Shared application state and serialized configuration mutations.
mod status;
use crate::{activity::OPERATIONS, config::Config, device::Device};
use serde::Serialize;
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
        self.save_config(&mut device, config)?;
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
        self.save_config(&mut device, config)
    }

    fn save_config(&self, device: &mut Device, config: Config) -> Result<(), String> {
        device
            .replace_config(config)
            .map_err(|error| error.to_string())?;
        // Let the save response complete before changing the browser's link.
        *self.wifi_change_after_ms.lock().unwrap() =
            crate::rtc_schedule::now_ms().saturating_add(1_000);
        Ok(())
    }
}
