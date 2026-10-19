//! Quote/0 appliance, bounded management paths, persistence, and hardware drivers.

pub mod activity;
pub mod app;
pub mod battery;
pub mod bluetooth;
pub mod bootstrap;
pub mod bootstrap_line;
pub mod bthome;
pub mod clock_face;
pub mod config;
pub mod device;
pub mod display;
pub mod embedded_graphics;
pub mod frame;
pub mod management;
pub mod network;
pub mod network_policy;
pub mod ota;
pub mod power;
pub mod power_policy;
pub mod pull;
pub mod pull_schedule;
pub mod refresh_policy;
pub mod rtc_schedule;
pub mod status_bar;
pub mod status_icons;
pub mod store;
pub mod usb_serial;
pub mod wall_clock;
pub mod wifi;

/// Perform the ESP-IDF runtime linkage required by every firmware binary.
///
/// A firmware entry point must call this once before it creates drivers.
pub fn link_patches() {
    esp_idf_sys::link_patches();
}
