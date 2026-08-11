//! The small device-owned status overlay described in the proposal.
//!
//! It deliberately uses compact text glyphs until the final one-bit icon set
//! is supplied. The bar never owns refresh scheduling; callers compose it only
//! when they have already decided to refresh the panel.

use crate::frame::{Frame, HEIGHT, WIDTH};

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
    pub fn overlay(&self, frame: &mut Frame) {
        for y in 0..HEIGHT {
            for x in CONTENT_WIDTH..WIDTH {
                frame.set_pixel(x, y, true);
            }
        }

        if let Some(marker) = self.battery {
            let text = match marker {
                BatteryMarker::Level(level) => match level.min(4) {
                    0 => "B0",
                    1 => "B1",
                    2 => "B2",
                    3 => "B3",
                    _ => "B4",
                },
                BatteryMarker::Charging => "B+",
                BatteryMarker::Critical => "B!",
            };
            draw_text(frame, 2, 4, text);
        }
        if let Some(marker) = self.wifi {
            let text = match marker {
                WifiMarker::Connected => "W+",
                WifiMarker::Connecting => "W~",
                WifiMarker::Failed => "W!",
            };
            draw_text(frame, 2, 26, text);
        }
        if self.usb_present {
            draw_text(frame, 2, 48, "U");
        }
        if self.bluetooth_present {
            draw_text(frame, 2, 70, "T");
        }
    }
}

/// A 3 x 5 placeholder font. Glyph pixels are black (`0`) on the white panel.
fn draw_text(frame: &mut Frame, x: usize, y: usize, text: &str) {
    for (index, ch) in text.chars().enumerate() {
        let glyph = glyph(ch);
        for (row, bits) in glyph.iter().copied().enumerate() {
            for column in 0..3 {
                if bits & (0b100 >> column) != 0 {
                    frame.set_pixel(x + index * 4 + column, y + row, false);
                }
            }
        }
    }
}

fn glyph(ch: char) -> [u8; 5] {
    match ch {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        'B' => [0b110, 0b101, 0b110, 0b101, 0b110],
        'W' => [0b101, 0b101, 0b101, 0b111, 0b101],
        'U' => [0b101, 0b101, 0b101, 0b101, 0b111],
        'T' => [0b111, 0b010, 0b010, 0b010, 0b010],
        '+' => [0b000, 0b010, 0b111, 0b010, 0b000],
        '!' => [0b010, 0b010, 0b010, 0b000, 0b010],
        '~' => [0b000, 0b000, 0b011, 0b110, 0b000],
        _ => [0; 5],
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
        assert_eq!(frame.pixel_is_white(CONTENT_WIDTH + 2, 4), Some(false));
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
}
