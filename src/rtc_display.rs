//! Last confirmed composed pixels, retained across deep sleep without flash wear.
#![cfg(any(target_os = "espidf", test))]
use crate::frame::Sha256Digest;

const MAGIC: u64 = 0x5155_4453_504c_0001;

#[derive(Clone, Copy)]
#[repr(C)]
struct SavedDisplay {
    magic_version: u64,
    composed: Sha256Digest,
    checksum: u64,
}

impl SavedDisplay {
    const EMPTY: Self = Self {
        magic_version: 0,
        composed: [0; 32],
        checksum: 0,
    };

    fn checksum(digest: Sha256Digest) -> u64 {
        digest.chunks_exact(8).fold(MAGIC, |sum, bytes| {
            sum ^ u64::from_le_bytes(bytes.try_into().unwrap())
        })
    }

    fn new(composed: Sha256Digest) -> Self {
        Self {
            magic_version: MAGIC,
            composed,
            checksum: Self::checksum(composed),
        }
    }

    fn digest(self) -> Option<Sha256Digest> {
        (self.magic_version == MAGIC && self.checksum == Self::checksum(self.composed))
            .then_some(self.composed)
    }
}

#[cfg(target_os = "espidf")]
#[link_section = ".rtc_noinit"]
static mut SAVED: SavedDisplay = SavedDisplay::EMPTY;

#[cfg(target_os = "espidf")]
pub fn load() -> Option<Sha256Digest> {
    if unsafe { esp_idf_sys::esp_reset_reason() }
        != esp_idf_sys::esp_reset_reason_t_ESP_RST_DEEPSLEEP
    {
        return None;
    }
    // Only the renderer accesses this slot. Cold boots and OTA must establish
    // a physical baseline even if RTC memory happens to contain an old digest.
    unsafe { core::ptr::addr_of!(SAVED).read_volatile() }.digest()
}

#[cfg(target_os = "espidf")]
pub fn save(digest: Option<Sha256Digest>) {
    unsafe {
        core::ptr::addr_of_mut!(SAVED)
            .write_volatile(digest.map(SavedDisplay::new).unwrap_or(SavedDisplay::EMPTY));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_digest_rejects_empty_corrupt_and_wrong_version_records() {
        let digest = core::array::from_fn(|index| index as u8);
        let saved = SavedDisplay::new(digest);
        assert_eq!(core::mem::size_of::<SavedDisplay>(), 48);
        assert_eq!(saved.digest(), Some(digest));
        assert_eq!(SavedDisplay::EMPTY.digest(), None);
        for byte in 0..digest.len() {
            let mut corrupt = saved;
            corrupt.composed[byte] ^= 1;
            assert_eq!(corrupt.digest(), None);
        }
        let mut old_version = saved;
        old_version.magic_version ^= 1;
        assert_eq!(old_version.digest(), None);
        let mut bad_checksum = saved;
        bad_checksum.checksum ^= 1;
        assert_eq!(bad_checksum.digest(), None);
    }
}
