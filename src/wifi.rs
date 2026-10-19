//! ESP32-C3 Wi-Fi station driver.
//!
//! This module owns only radio mechanics. Credential storage, retry policy,
//! and application networking belong in `network`.

use esp_idf_hal::modem::Modem;
use esp_idf_hal::sys::EspError;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{
    AccessPointConfiguration, AuthMethod, ClientConfiguration, Configuration, EspWifi,
};

pub struct WifiStation<'d> {
    inner: EspWifi<'d>,
}

impl<'d> WifiStation<'d> {
    pub fn new(
        modem: Modem<'d>,
        event_loop: EspSystemEventLoop,
        nvs: Option<EspDefaultNvsPartition>,
    ) -> Result<Self, EspError> {
        Ok(Self {
            inner: EspWifi::new(modem, event_loop, nvs)?,
        })
    }

    /// Configure a WPA/WPA2 station. Connecting and waiting for DHCP is left
    /// to the caller so timeout and retry policy stay outside the driver.
    pub fn configure(&mut self, ssid: &str, password: &str) -> Result<(), String> {
        let auth_method = if password.is_empty() {
            AuthMethod::None
        } else {
            AuthMethod::WPA2Personal
        };
        let ssid = ssid
            .try_into()
            .map_err(|_| "SSID exceeds 32 bytes".to_owned())?;
        let password = password
            .try_into()
            .map_err(|_| "password exceeds 64 bytes".to_owned())?;
        self.inner
            .set_configuration(&Configuration::Client(ClientConfiguration {
                ssid,
                password,
                auth_method,
                ..Default::default()
            }))
            .map_err(|error| error.to_string())
    }

    pub fn configure_setup(&mut self, ssid: &str, password: &str) -> Result<(), String> {
        self.inner
            .set_configuration(&Configuration::AccessPoint(AccessPointConfiguration {
                ssid: ssid.try_into().map_err(|_| "Invalid setup SSID")?,
                password: password.try_into().map_err(|_| "Invalid setup password")?,
                auth_method: AuthMethod::WPA2Personal,
                max_connections: 2,
                ..Default::default()
            }))
            .map_err(|e| e.to_string())
    }
    pub fn ip(&self, setup: bool) -> Option<String> {
        let netif = if setup {
            self.inner.ap_netif()
        } else {
            self.inner.sta_netif()
        };
        netif.get_ip_info().ok().map(|info| info.ip.to_string())
    }

    pub fn start(&mut self) -> Result<(), EspError> {
        self.inner.start()
    }

    /// Park the station between pull attempts. The driver and NVS state
    /// survive; the radio does not draw power while idle.
    pub fn stop(&mut self) -> Result<(), EspError> {
        self.inner.stop()
    }

    pub fn connect(&mut self) -> Result<(), EspError> {
        self.inner.connect()
    }
    pub fn is_up(&self) -> Result<bool, EspError> {
        self.inner.is_up()
    }
}
