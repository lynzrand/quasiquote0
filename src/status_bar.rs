//! The small device-owned status overlay described in the proposal.
//!
//! The supplied one-bit icons are compiled into the firmware. The bar never
//! owns refresh scheduling; callers compose a complete snapshot for content
//! changes and each minute heartbeat.

use crate::clock_face::ClockFace;
use crate::frame::{Frame, HEIGHT, WIDTH};
use crate::status_icons::{
    Icon, BATTERY_0, BATTERY_100, BATTERY_25, BATTERY_50, BATTERY_75, BATTERY_CRITICAL, BLUETOOTH,
    CHARGING, USB_POWER, WIFI_CONNECTED, WIFI_DISCONNECTED, WIFI_PULL_FAILED,
};
use core::convert::Infallible;
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    text::Text,
};

pub const WIDTH_PIXELS: usize = 16;
pub const CONTENT_WIDTH: usize = WIDTH - WIDTH_PIXELS;

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
        percent.map(|percent| BatteryMarker::Level(((percent.min(100) + 24) / 25) as u8))
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
            draw_icon(frame, 0, battery_icon(marker));
        }
        if let Some(marker) = self.wifi {
            draw_icon(frame, 24, wifi_icon(marker));
        }
        if self.usb_present {
            draw_icon(frame, 48, USB_POWER);
        }
        if self.bluetooth_present {
            draw_icon(frame, 72, BLUETOOTH);
        }
        let text = ClockFace::at(self.unix_seconds.unwrap_or(0)).text();
        Text::new(
            &text,
            Point::new(0, 7),
            MonoTextStyle::new(&FONT_6X10, BinaryColor::Off),
        )
        .draw(&mut ClockTarget(frame))
        .unwrap();
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
        // There is no distinct association-progress artwork; disconnected is
        // the least misleading snapshot while association is incomplete.
        WifiMarker::Disconnected | WifiMarker::Connecting => WIFI_DISCONNECTED,
        WifiMarker::Failed => WIFI_PULL_FAILED,
    }
}

/// Rotate HH:MM into the strip using the same orientation as the icon art.
/// Its 30x10 source fits in the bottom 30 rows with a three-pixel side margin.
struct ClockTarget<'a>(&'a mut Frame);

impl OriginDimensions for ClockTarget<'_> {
    fn size(&self) -> Size {
        Size::new(30, 10)
    }
}

impl DrawTarget for ClockTarget<'_> {
    type Color = BinaryColor;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            if (0..30).contains(&point.x) && (0..10).contains(&point.y) {
                self.0.set_pixel(
                    CONTENT_WIDTH + 3 + 9 - point.y as usize,
                    HEIGHT - 30 + point.x as usize,
                    color == BinaryColor::On,
                );
            }
        }
        Ok(())
    }
}

/// Draw a sprite rotated a quarter-turn counter-clockwise. The strip is a
/// narrow vertical band on the panel, but the icon art is authored for the
/// device's landscape reading orientation, so battery terminals point right.
fn draw_icon(frame: &mut Frame, y: usize, icon: Icon) {
    for (row_index, row) in icon.iter().copied().enumerate() {
        for column in 0..WIDTH_PIXELS {
            if row & (0x8000 >> column) != 0 {
                frame.set_pixel(
                    CONTENT_WIDTH + (WIDTH_PIXELS - 1 - row_index),
                    y + column,
                    false,
                );
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
        // Battery-75's terminal nub is sprite row 1, columns 6-9; rotated a
        // quarter-turn counter-clockwise it lands at canvas x=CONTENT_WIDTH+14.
        assert_eq!(frame.pixel_is_white(CONTENT_WIDTH + 14, 6), Some(false));
        assert_eq!(frame.pixel_is_white(CONTENT_WIDTH + 14, 9), Some(false));
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
        for y in 0..88 {
            for x in CONTENT_WIDTH..WIDTH {
                assert_eq!(frame.pixel_is_white(x, y), Some(true));
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
        for y in 0..HEIGHT - 30 {
            for x in 0..WIDTH {
                assert_eq!(first.pixel_is_white(x, y), next.pixel_is_white(x, y));
            }
        }
        let mut unknown = Frame::white();
        StatusBar::default().overlay(&mut unknown);
        assert_ne!(unknown.as_bytes(), first.as_bytes());
    }
}
