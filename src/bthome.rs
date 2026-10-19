//! Driver-independent BTHome v2 telemetry encoding.

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
    fn status_flags_use_the_existing_bthome_object_ids() {
        let payload = BthomeAdvertisement {
            battery_percent: 9,
            battery_mv: 3300,
            battery_low: true,
            charging: true,
            wifi_connected: true,
            usb_present: true,
        }
        .encode();
        assert_eq!(
            payload,
            [
                2, 1, 6, 17, 0x16, 0xd2, 0xfc, 0x40, 1, 9, 0x0c, 0xe4, 0x0c, 0x15, 1, 0x16, 1,
                0x19, 1, 0x24, 1
            ]
        );
    }

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
