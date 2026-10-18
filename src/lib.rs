//! Hardware drivers for the MindReset Quote/0 (ESP32-C3).
//!
//! This crate deliberately contains no application, protocol, persistence, or
//! policy loop. A future firmware crate should construct these drivers from
//! `esp_idf_hal::peripherals::Peripherals` and own its scheduling itself.

pub mod battery;
pub mod bluetooth;
pub mod config;
pub mod device;
pub mod display;
pub mod embedded_graphics;
pub mod frame;
pub mod i2c_probe;
pub mod jsonrpc;
pub mod ota;
pub mod power;
pub mod power_policy;
pub mod pull;
pub mod pull_schedule;
pub mod rpc;
pub mod status_bar;
pub mod status_icons;
pub mod status_screen;
pub mod store;
pub mod usb_serial;
pub mod wifi;
pub mod yamux;

/// Perform the ESP-IDF runtime linkage required by every firmware binary.
///
/// A firmware entry point must call this once before it creates drivers.
pub fn link_patches() {
    esp_idf_sys::link_patches();
}
