//! ESP32-C3 Wi-Fi station driver.
//!
//! This module owns only radio mechanics. Credential storage, retry policy,
//! and application networking belong in the eventual firmware crate.

use esp_idf_hal::modem::Modem;
use esp_idf_hal::sys::EspError;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{ClientConfiguration, Configuration, EspWifi};

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
                ..Default::default()
            }))
            .map_err(|error| error.to_string())
    }

    pub fn start(&mut self) -> Result<(), EspError> {
        self.inner.start()
    }
    pub fn connect(&mut self) -> Result<(), EspError> {
        self.inner.connect()
    }
    pub fn disconnect(&mut self) -> Result<(), EspError> {
        self.inner.disconnect()
    }
    pub fn is_up(&self) -> Result<bool, EspError> {
        self.inner.is_up()
    }
    pub fn inner(&mut self) -> &mut EspWifi<'d> {
        &mut self.inner
    }
}
