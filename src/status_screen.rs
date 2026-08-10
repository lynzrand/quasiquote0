//! Rendering for the stand-alone ten-second status display.

use crate::battery::BatteryStatus;
use crate::display::Frame;
use crate::embedded_graphics::Framebuffer;
use embedded_graphics::draw_target::DrawTarget;
use embedded_graphics::geometry::Point;
use embedded_graphics::mono_font::{ascii::FONT_6X10, MonoTextStyle};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::{Alignment, Text};

pub fn render(frame: &mut Frame, wifi_up: bool, bluetooth_ready: bool, battery: BatteryStatus) {
    let mut target = Framebuffer::new(frame);
    target.clear(BinaryColor::On).ok();
    let ink = BinaryColor::Off;
    Rectangle::new(Point::new(4, 4), Size::new(144, 288))
        .into_styled(PrimitiveStyle::with_stroke(ink, 2))
        .draw(&mut target)
        .ok();

    let title = MonoTextStyle::new(&embedded_graphics::mono_font::ascii::FONT_10X20, ink);
    let body = MonoTextStyle::new(&FONT_6X10, ink);
    Text::with_alignment("Quote/0", Point::new(76, 35), title, Alignment::Center)
        .draw(&mut target)
        .ok();
    Text::with_alignment(
        if wifi_up {
            "WiFi: connected"
        } else {
            "WiFi: disconnected"
        },
        Point::new(76, 82),
        body,
        Alignment::Center,
    )
    .draw(&mut target)
    .ok();
    Text::with_alignment(
        if bluetooth_ready {
            "Bluetooth: ready"
        } else {
            "Bluetooth: starting"
        },
        Point::new(76, 106),
        body,
        Alignment::Center,
    )
    .draw(&mut target)
    .ok();
    let battery_line = match (battery.percent, battery.millivolts) {
        (Some(percent), Some(mv)) => format!("Battery: {percent}% ({mv} mV)"),
        _ => "Battery: unavailable".to_owned(),
    };
    Text::with_alignment(&battery_line, Point::new(76, 130), body, Alignment::Center)
        .draw(&mut target)
        .ok();
    Text::with_alignment(
        if battery.usb_present {
            "Power: USB"
        } else {
            "Power: battery"
        },
        Point::new(76, 154),
        body,
        Alignment::Center,
    )
    .draw(&mut target)
    .ok();
    Text::with_alignment(
        "Refresh: 10 seconds",
        Point::new(76, 260),
        body,
        Alignment::Center,
    )
    .draw(&mut target)
    .ok();
}
