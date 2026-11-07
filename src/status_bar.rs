//! The small device-owned status overlay described in the proposal.
//!
//! The one-bit icons are compiled into the firmware. The bar never
//! owns refresh scheduling; callers compose a complete snapshot for content
//! changes and each minute heartbeat.

use crate::clock_face::ClockFace;
use crate::embedded_graphics::LandscapeFramebuffer;
use crate::frame::{Frame, HEIGHT, WIDTH};
use crate::status_icons::{
    Icon, BATTERY_0, BATTERY_100, BATTERY_25, BATTERY_50, BATTERY_75, BATTERY_CRITICAL, BLUETOOTH,
    CHARGING, USB_POWER, WIFI_CONNECTED, WIFI_CONNECTING, WIFI_DISCONNECTED, WIFI_PULL_FAILED,
};
use embedded_graphics::{
    mono_font::{ascii::FONT_7X13_BOLD, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Baseline, Text},
};

pub const WIDTH_PIXELS: usize = 16;
pub const CONTENT_WIDTH: usize = WIDTH - WIDTH_PIXELS;
const CLOCK_TOP: usize = 112;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BatteryMarker {
    Level(u8),
    Charging,
    Critical,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WifiMarker {
    Disconnected,
    Connected,
    Connecting,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StatusBar {
    pub battery: Option<BatteryMarker>,
    pub wifi: Option<WifiMarker>,
    pub usb_present: bool,
    pub bluetooth_present: bool,
    /// Current UTC, sampled with the rest of the snapshot. Unknown renders --:--.
    pub unix_seconds: Option<i64>,
}

impl StatusBar {
    pub fn from_battery_percent(percent: Option<u32>, charging: bool) -> Option<BatteryMarker> {
        if charging {
            return Some(BatteryMarker::Charging);
        }
        percent.map(|percent| BatteryMarker::Level(percent.min(100).div_ceil(25) as u8))
    }

    /// Replace the whole status region. This makes the producer's reserved
    /// columns irrelevant and prevents stale glyph pixels from surviving a
    /// changed status snapshot.
    ///
    /// The strip occupies the canvas's last columns, which the projection
    /// places along the viewer's right edge of the mounted panel.
    pub fn overlay(&self, frame: &mut Frame) {
        for y in 0..HEIGHT {
            for x in CONTENT_WIDTH..WIDTH {
                frame.set_pixel(x, y, true);
            }
        }

        if let Some(marker) = self.battery {
            draw_icon(frame, 4, battery_icon(marker));
        }
        if let Some(marker) = self.wifi {
            draw_icon(frame, 28, wifi_icon(marker));
        }
        if self.usb_present {
            draw_icon(frame, 52, USB_POWER);
        }
        if self.bluetooth_present {
            draw_icon(frame, 76, BLUETOOTH);
        }
        for x in 4..12 {
            frame.set_pixel(CONTENT_WIDTH + x, 103, false);
        }
        // ClockFace always returns five ASCII characters: HH:MM or --:--.
        // Keep the digits upright, with hours above minutes and a fixed colon.
        let text = ClockFace::at(self.unix_seconds.unwrap_or(0)).text();
        let style = MonoTextStyle::new(&FONT_7X13_BOLD, BinaryColor::Off);
        for (digits, y) in [(&text[..2], CLOCK_TOP), (&text[3..], CLOCK_TOP + 22)] {
            Text::with_baseline(
                digits,
                Point::new((CONTENT_WIDTH + 1) as i32, y as i32),
                style,
                Baseline::Top,
            )
            .draw(&mut LandscapeFramebuffer(frame))
            .unwrap();
        }
        for y in [CLOCK_TOP + 14, CLOCK_TOP + 18] {
            for dy in 0..2 {
                for x in 7..9 {
                    frame.set_pixel(CONTENT_WIDTH + x, y + dy, false);
                }
            }
        }
    }
}

fn battery_icon(marker: BatteryMarker) -> Icon {
    match marker {
        BatteryMarker::Level(level) => match level.min(4) {
            0 => BATTERY_0,
            1 => BATTERY_25,
            2 => BATTERY_50,
            3 => BATTERY_75,
            _ => BATTERY_100,
        },
        BatteryMarker::Charging => CHARGING,
        BatteryMarker::Critical => BATTERY_CRITICAL,
    }
}

fn wifi_icon(marker: WifiMarker) -> Icon {
    match marker {
        WifiMarker::Connected => WIFI_CONNECTED,
        WifiMarker::Disconnected => WIFI_DISCONNECTED,
        // A static progress glyph avoids animation-only display refreshes.
        WifiMarker::Connecting => WIFI_CONNECTING,
        WifiMarker::Failed => WIFI_PULL_FAILED,
    }
}

/// Draw artwork upright in the final landscape coordinates. Projection into
/// controller order happens only at the EPD boundary.
fn draw_icon(frame: &mut Frame, y: usize, icon: Icon) {
    for (row_index, row) in icon.iter().copied().enumerate() {
        for column in 0..WIDTH_PIXELS {
            if row & (0x8000 >> column) != 0 {
                frame.set_pixel(CONTENT_WIDTH + column, y + row_index, false);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_keeps_content_and_replaces_reserved_columns() {
        let mut frame = Frame::black();
        let status = StatusBar {
            battery: Some(BatteryMarker::Level(3)),
            usb_present: true,
            ..Default::default()
        };
        status.overlay(&mut frame);

        assert_eq!(frame.pixel_is_white(CONTENT_WIDTH - 1, 0), Some(false));
        assert_eq!(frame.pixel_is_white(CONTENT_WIDTH, HEIGHT - 1), Some(true));
        // Upright battery cap: sprite row 1, columns 6-9, placed four rows down.
        assert_eq!(frame.pixel_is_white(CONTENT_WIDTH + 6, 5), Some(false));
        assert_eq!(frame.pixel_is_white(CONTENT_WIDTH + 9, 5), Some(false));
        assert_eq!(frame.pixel_is_white(CONTENT_WIDTH + 5, 5), Some(true));
    }

    #[test]
    fn battery_levels_are_quantized_to_five_markers() {
        assert_eq!(
            StatusBar::from_battery_percent(Some(0), false),
            Some(BatteryMarker::Level(0))
        );
        assert_eq!(
            StatusBar::from_battery_percent(Some(100), false),
            Some(BatteryMarker::Level(4))
        );
    }

    #[test]
    fn every_snapshot_preserves_all_content_pixels_and_clears_old_status() {
        let original = Frame::black();
        let mut frame = original.clone();
        StatusBar {
            battery: Some(BatteryMarker::Charging),
            wifi: Some(WifiMarker::Failed),
            usb_present: true,
            bluetooth_present: true,
            unix_seconds: Some(1_577_836_800),
        }
        .overlay(&mut frame);
        for y in 0..HEIGHT {
            for x in 0..CONTENT_WIDTH {
                assert_eq!(frame.pixel_is_white(x, y), original.pixel_is_white(x, y));
            }
        }
        StatusBar::default().overlay(&mut frame);
        let mut cleared = Frame::white();
        StatusBar::default().overlay(&mut cleared);
        for y in 0..HEIGHT {
            for x in CONTENT_WIDTH..WIDTH {
                assert_eq!(frame.pixel_is_white(x, y), cleared.pixel_is_white(x, y));
            }
        }
    }

    #[test]
    fn minute_clock_changes_without_changing_the_content_or_other_icons() {
        let mut first = Frame::white();
        let mut next = Frame::white();
        let seconds = 1_577_836_800;
        StatusBar {
            unix_seconds: Some(seconds),
            ..Default::default()
        }
        .overlay(&mut first);
        StatusBar {
            unix_seconds: Some(seconds + 60),
            ..Default::default()
        }
        .overlay(&mut next);
        assert_ne!(first.as_bytes(), next.as_bytes());
        let mut same_minute = Frame::white();
        StatusBar {
            unix_seconds: Some(seconds + 59),
            ..Default::default()
        }
        .overlay(&mut same_minute);
        assert_eq!(first.as_bytes(), same_minute.as_bytes());
        for y in 0..CLOCK_TOP {
            for x in 0..WIDTH {
                assert_eq!(first.pixel_is_white(x, y), next.pixel_is_white(x, y));
            }
        }
        let mut unknown = Frame::white();
        StatusBar::default().overlay(&mut unknown);
        assert_ne!(unknown.as_bytes(), first.as_bytes());
    }
}
