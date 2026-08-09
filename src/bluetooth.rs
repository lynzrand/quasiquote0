//! Bluetooth LE primitives for Quote/0.
//!
//! The board uses the ESP-IDF NimBLE host. Host-task ownership is intentionally
//! not started here: an application must decide when the radio is enabled and
//! own the host lifecycle. `BthomeAdvertisement` provides the driver-neutral
//! payload for a non-connectable BTHome v2 broadcaster.

pub const BTHOME_PAYLOAD_LEN: usize = 21;

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
