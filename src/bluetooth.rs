//! Bluetooth LE primitives for Quote/0.
//!
//! The board uses the ESP-IDF NimBLE host. `Bluetooth` owns host startup;
//! advertising policy remains in the application. `BthomeAdvertisement`
//! provides a driver-neutral payload for a non-connectable BTHome v2
//! broadcaster. The intended GATT push channel is specified in
//! `docs/PROTOCOL.md` but not implemented yet.

use core::sync::atomic::{AtomicBool, Ordering};

pub const BTHOME_PAYLOAD_LEN: usize = 21;
const ADVERTISEMENT_DURATION_MS: i32 = 60_000;

static HOST_SYNCED: AtomicBool = AtomicBool::new(false);

/// A running NimBLE host task.
pub struct Bluetooth;

impl Bluetooth {
    pub fn start() -> Result<Self, esp_idf_hal::sys::EspError> {
        unsafe {
            esp_idf_sys::esp!(esp_idf_sys::nimble_port_init())?;
            esp_idf_sys::ble_hs_cfg.sync_cb = Some(on_sync);
            esp_idf_sys::ble_hs_cfg.reset_cb = Some(on_reset);
            esp_idf_sys::nimble_port_freertos_init(Some(host_task));
        }
        Ok(Self)
    }

    pub fn is_ready(&self) -> bool {
        HOST_SYNCED.load(Ordering::SeqCst)
    }

    pub fn is_advertising(&self) -> bool {
        self.is_ready() && unsafe { esp_idf_sys::ble_gap_adv_active() != 0 }
    }

    /// Keep a non-connectable BTHome v2 beacon on air while the device is
    /// awake. Calling repeatedly is cheap: an unchanged payload with an
    /// active advertisement is a no-op, and a changed payload restarts the
    /// procedure. NimBLE allows restarts because callers run outside the
    /// host task; the previous procedure is stopped first.
    pub fn advertise(&self, snapshot: &BthomeAdvertisement) {
        if !self.is_ready() {
            return;
        }
        let payload = snapshot.encode();
        if self.is_advertising() && payload == last_payload() {
            return;
        }
        unsafe {
            if esp_idf_sys::ble_gap_adv_active() != 0 {
                let _ = esp_idf_sys::ble_gap_adv_stop();
            }
            if esp_idf_sys::ble_gap_adv_set_data(payload.as_ptr(), payload.len() as i32) != 0 {
                return;
            }
            let mut params: esp_idf_sys::ble_gap_adv_params = core::mem::zeroed();
            params.conn_mode = esp_idf_sys::BLE_GAP_CONN_MODE_NON as u8;
            params.disc_mode = esp_idf_sys::BLE_GAP_DISC_MODE_GEN as u8;
            let result = esp_idf_sys::ble_gap_adv_start(
                esp_idf_sys::BLE_OWN_ADDR_PUBLIC as u8,
                core::ptr::null(),
                ADVERTISEMENT_DURATION_MS,
                &params,
                Some(gap_event),
                core::ptr::null_mut(),
            );
            if result == 0 {
                store_last_payload(&payload);
            }
        }
    }
}

fn last_payload() -> [u8; BTHOME_PAYLOAD_LEN] {
    *LAST_PAYLOAD.lock().unwrap()
}

fn store_last_payload(payload: &[u8; BTHOME_PAYLOAD_LEN]) {
    *LAST_PAYLOAD.lock().unwrap() = *payload;
}

static LAST_PAYLOAD: std::sync::Mutex<[u8; BTHOME_PAYLOAD_LEN]> =
    std::sync::Mutex::new([0; BTHOME_PAYLOAD_LEN]);

extern "C" fn on_sync() {
    HOST_SYNCED.store(true, Ordering::SeqCst);
}

extern "C" fn on_reset(_reason: i32) {
    HOST_SYNCED.store(false, Ordering::SeqCst);
}

extern "C" fn host_task(_param: *mut core::ffi::c_void) {
    unsafe {
        esp_idf_sys::nimble_port_run();
        esp_idf_sys::nimble_port_freertos_deinit();
    }
}

unsafe extern "C" fn gap_event(
    _event: *mut esp_idf_sys::ble_gap_event,
    _arg: *mut core::ffi::c_void,
) -> i32 {
    0
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BthomeAdvertisement {
    pub battery_percent: u8,
    pub battery_mv: u16,
    pub battery_low: bool,
    pub charging: bool,
    pub wifi_connected: bool,
    pub usb_present: bool,
}

impl BthomeAdvertisement {
    pub fn encode(self) -> [u8; BTHOME_PAYLOAD_LEN] {
        let mut d = [0; BTHOME_PAYLOAD_LEN];
        d[0..3].copy_from_slice(&[2, 1, 0x06]); // LE general discoverable
        d[3..8].copy_from_slice(&[17, 0x16, 0xd2, 0xfc, 0x40]); // BTHome v2
        d[8..21].copy_from_slice(&[
            0x01,
            self.battery_percent,
            0x0c,
            self.battery_mv as u8,
            (self.battery_mv >> 8) as u8,
            0x15,
            self.battery_low as u8,
            0x16,
            self.charging as u8,
            0x19,
            self.wifi_connected as u8,
            0x24,
            self.usb_present as u8,
        ]);
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bthome_layout_is_stable() {
        let payload = BthomeAdvertisement {
            battery_percent: 42,
            battery_mv: 3700,
            ..Default::default()
        }
        .encode();
        assert_eq!(&payload[..8], &[2, 1, 6, 17, 0x16, 0xd2, 0xfc, 0x40]);
        assert_eq!(&payload[8..13], &[1, 42, 0x0c, 0x74, 0x0e]);
    }
}
