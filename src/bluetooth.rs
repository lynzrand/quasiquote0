//! NimBLE host lifecycle and non-connectable BTHome advertising.

use core::sync::atomic::{AtomicBool, Ordering};

use crate::bthome::{BthomeAdvertisement, BTHOME_PAYLOAD_LEN};
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
