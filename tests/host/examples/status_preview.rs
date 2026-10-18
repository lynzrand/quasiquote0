//! Canonical fixture for the actual status renderer, also uploadable via USB.
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Alignment, Text},
};
use quasiquote0_host_tests::{
    embedded_graphics::LandscapeFramebuffer,
    frame::Frame,
    status_bar::{BatteryMarker, StatusBar, WifiMarker},
};

fn main() {
    let mut frame = Frame::white();
    let mut target = LandscapeFramebuffer(&mut frame);
    let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);
    for (text, y) in [
        ("Quote/0", 27),
        ("Status bar heartbeat", 58),
        ("Content stays unchanged", 87),
        ("Clock updates each minute", 112),
    ] {
        Text::with_alignment(text, Point::new(140, y), style, Alignment::Center)
            .draw(&mut target)
            .unwrap();
    }
    // Clear the narrow strip so the file can also be the producer's frame.
    // The firmware replaces this strip with its own fresh snapshot on upload.
    if !std::env::args().any(|arg| arg == "--content-only") {
        StatusBar {
            battery: Some(BatteryMarker::Level(3)),
            wifi: Some(WifiMarker::Connected),
            usb_present: true,
            bluetooth_present: true,
            unix_seconds: Some(1_577_836_800 + 12 * 3600 + 58 * 60),
        }
        .overlay(&mut frame);
    }
    let path = std::env::args_os().nth(1).expect("output .bin path");
    std::fs::write(path, frame.as_bytes()).unwrap();
}
