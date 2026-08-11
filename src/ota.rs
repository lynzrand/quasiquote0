//! Bounded USB OTA receiver for an ESP-IDF application image.
//!
//! This module never stores firmware in LittleFS or RAM. The caller provides a
//! blocking Yamux stream and the announced exact length/digest; chunks are
//! written directly to the inactive OTA partition. Selecting that validated
//! partition to boot is deliberately a separate final step, after LittleFS has
//! recorded the pending-upgrade marker.

use crate::frame::Sha256Digest;
use sha2::{Digest, Sha256};
use std::io::Read;

pub fn receive(
    stream: &mut impl Read,
    expected_bytes: usize,
    expected_digest: Sha256Digest,
) -> Result<(), String> {
    receive_with_progress(stream, expected_bytes, expected_digest, |_| {})
}

/// As [`receive`], but report bytes committed to the inactive OTA partition
/// after each bounded write. The callback must be short and non-blocking.
pub fn receive_with_progress(
    stream: &mut impl Read,
    expected_bytes: usize,
    expected_digest: Sha256Digest,
    mut progress: impl FnMut(usize),
) -> Result<(), String> {
    if expected_bytes == 0 {
        return Err("OTA image length must be non-zero".to_owned());
    }
    let target = unsafe { esp_idf_sys::esp_ota_get_next_update_partition(core::ptr::null()) };
    if target.is_null() {
        return Err("no inactive OTA partition is available".to_owned());
    }

    let mut handle: esp_idf_sys::esp_ota_handle_t = 0;
    esp_idf_sys::esp!(unsafe { esp_idf_sys::esp_ota_begin(target, expected_bytes, &mut handle) })
        .map_err(|error| error.to_string())?;

    let result = receive_inner(stream, expected_bytes, expected_digest, handle, &mut progress);
    match result {
        Ok(()) => esp_idf_sys::esp!(unsafe { esp_idf_sys::esp_ota_end(handle) })
            .map_err(|error| error.to_string()),
        Err(error) => {
            let _ = unsafe { esp_idf_sys::esp_ota_abort(handle) };
            Err(error)
        }
    }
}

/// Select the just-validated inactive slot. Call this only after a durable
/// pending-upgrade marker has been committed.
pub fn select_staged_partition() -> Result<(), String> {
    let target = unsafe { esp_idf_sys::esp_ota_get_next_update_partition(core::ptr::null()) };
    if target.is_null() {
        return Err("no inactive OTA partition is available".to_owned());
    }
    esp_idf_sys::esp!(unsafe { esp_idf_sys::esp_ota_set_boot_partition(target) })
        .map_err(|error| error.to_string())
}

fn receive_inner(
    stream: &mut impl Read,
    expected_bytes: usize,
    expected_digest: Sha256Digest,
    handle: esp_idf_sys::esp_ota_handle_t,
    progress: &mut impl FnMut(usize),
) -> Result<(), String> {
    let mut remaining = expected_bytes;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 512];
    while remaining != 0 {
        let chunk_len = remaining.min(buffer.len());
        let count = stream
            .read(&mut buffer[..chunk_len])
            .map_err(|error| error.to_string())?;
        if count == 0 {
            return Err("OTA stream ended before the announced length".to_owned());
        }
        esp_idf_sys::esp!(unsafe {
            esp_idf_sys::esp_ota_write(handle, buffer.as_ptr().cast(), count)
        })
        .map_err(|error| error.to_string())?;
        hasher.update(&buffer[..count]);
        remaining -= count;
        progress(expected_bytes - remaining);
    }
    let actual: Sha256Digest = hasher.finalize().into();
    if actual != expected_digest {
        return Err("OTA digest does not match announcement".to_owned());
    }
    Ok(())
}

/// Mark an OTA-selected image valid once its boot self-tests have completed.
pub fn confirm_running_image() -> Result<(), String> {
    esp_idf_sys::esp!(unsafe { esp_idf_sys::esp_ota_mark_app_valid_cancel_rollback() })
        .map_err(|error| error.to_string())
}

pub fn running_partition_label() -> Option<String> {
    let partition = unsafe { esp_idf_sys::esp_ota_get_running_partition() };
    if partition.is_null() {
        return None;
    }
    let label = unsafe { &(*partition).label };
    let bytes: Vec<u8> = label
        .iter()
        .copied()
        .take_while(|byte| *byte != 0)
        .map(|byte| byte as u8)
        .collect();
    String::from_utf8(bytes).ok()
}
