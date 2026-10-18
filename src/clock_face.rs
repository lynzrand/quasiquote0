//! Minute-only, landscape clock face. UTC is stored in the RTC; display is UTC+8.

use crate::embedded_graphics::LandscapeFramebuffer;
use crate::frame::Frame;
use embedded_graphics::mono_font::{ascii::FONT_6X10, MonoTextStyle};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::{Alignment, Text};
use serde::Serialize;

pub const UTC_OFFSET_MINUTES: i64 = 8 * 60;
pub const MIN_UNIX_SECONDS: i64 = 1_577_836_800; // 2020-01-01 UTC
pub const MAX_UNIX_SECONDS: i64 = 4_102_444_800; // 2100-01-01 UTC

pub fn valid_time(seconds: i64) -> bool {
    (MIN_UNIX_SECONDS..MAX_UNIX_SECONDS).contains(&seconds)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum ClockFace {
    WaitingForTime,
    Minute { hour: u8, minute: u8 },
}

impl ClockFace {
    pub fn at(unix_seconds: i64) -> Self {
        if !valid_time(unix_seconds) {
            return Self::WaitingForTime;
        }
        let local = (unix_seconds + UTC_OFFSET_MINUTES * 60).rem_euclid(24 * 60 * 60);
        Self::Minute {
            hour: (local / 3600) as u8,
            minute: ((local / 60) % 60) as u8,
        }
    }

    pub fn text(self) -> String {
        match self {
            Self::WaitingForTime => "--:--".to_owned(),
            Self::Minute { hour, minute } => format!("{hour:02}:{minute:02}"),
        }
    }
}

pub fn seconds_until_next_minute(unix_seconds: i64) -> u64 {
    (60 - unix_seconds.rem_euclid(60)) as u64
}

fn rectangle(frame: &mut Frame, x: usize, y: usize, width: usize, height: usize) {
    for py in y..y + height {
        for px in x..x + width {
            frame.set_pixel(px, py, false);
        }
    }
}

pub fn render(face: ClockFace) -> Frame {
    let mut frame = Frame::white();
    let digits = match face {
        ClockFace::Minute { hour, minute } => [hour / 10, hour % 10, minute / 10, minute % 10],
        ClockFace::WaitingForTime => [10; 4],
    };
    // A B C D E F G; the final entry is the unsynchronized dash.
    const MASKS: [u8; 11] = [
        0x3f, 0x06, 0x5b, 0x4f, 0x66, 0x6d, 0x7d, 0x07, 0x7f, 0x6f, 0x40,
    ];
    const SEGMENTS: [(usize, usize, usize, usize); 7] = [
        (6, 0, 28, 6),
        (34, 6, 6, 32),
        (34, 44, 6, 32),
        (6, 76, 28, 6),
        (0, 44, 6, 32),
        (0, 6, 6, 32),
        (6, 38, 28, 6),
    ];
    for (digit, x) in digits.into_iter().zip([35, 85, 171, 221]) {
        for (bit, (dx, dy, width, height)) in SEGMENTS.into_iter().enumerate() {
            if MASKS[digit as usize] & (1 << bit) != 0 {
                rectangle(&mut frame, x + dx, 32 + dy, width, height);
            }
        }
    }
    rectangle(&mut frame, 145, 53, 6, 6);
    rectangle(&mut frame, 145, 89, 6, 6);
    let mut target = LandscapeFramebuffer(&mut frame);
    let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
    Text::with_alignment("Quote/0", Point::new(148, 17), style, Alignment::Center)
        .draw(&mut target)
        .unwrap();
    Text::with_alignment(
        if face == ClockFace::WaitingForTime {
            "Waiting for time"
        } else {
            "HONG KONG  UTC+8"
        },
        Point::new(148, 140),
        style,
        Alignment::Center,
    )
    .draw(&mut target)
    .unwrap();
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hong_kong_midnight_rollover_and_minute_boundaries() {
        let midnight_utc = MIN_UNIX_SECONDS;
        assert_eq!(ClockFace::at(midnight_utc).text(), "08:00");
        assert_eq!(ClockFace::at(midnight_utc + 16 * 3600 - 1).text(), "23:59");
        assert_eq!(ClockFace::at(midnight_utc + 16 * 3600).text(), "00:00");
        assert_eq!(seconds_until_next_minute(midnight_utc), 60);
        assert_eq!(seconds_until_next_minute(midnight_utc + 59), 1);
    }

    #[test]
    fn seconds_do_not_change_the_display_face() {
        assert_eq!(
            ClockFace::at(MIN_UNIX_SECONDS),
            ClockFace::at(MIN_UNIX_SECONDS + 59)
        );
        assert_ne!(
            ClockFace::at(MIN_UNIX_SECONDS + 59),
            ClockFace::at(MIN_UNIX_SECONDS + 60)
        );
        assert_eq!(ClockFace::at(0), ClockFace::WaitingForTime);
    }

    #[test]
    fn render_uses_landscape_and_leaves_the_outer_margin_white() {
        let frame = render(ClockFace::at(MIN_UNIX_SECONDS));
        assert_eq!(frame.pixel_is_white(145, 53), Some(false));
        assert_eq!(frame.pixel_is_white(0, 0), Some(true));
        assert_eq!(frame.pixel_is_white(295, 151), Some(true));
        assert!(frame.as_bytes().iter().any(|b| *b != 0xff));
    }
}
