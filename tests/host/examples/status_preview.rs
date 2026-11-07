//! Render the production overlay as PBM files for pixel-accurate design review.
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    text::Text,
};
use quasiquote0_host_tests::{
    embedded_graphics::LandscapeFramebuffer,
    frame::Frame,
    status_bar::{BatteryMarker, StatusBar, WifiMarker},
};
use std::{fs, io::Write, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/status-preview"));
    fs::create_dir_all(&output)?;
    let normal = StatusBar {
        battery: Some(BatteryMarker::Level(3)),
        wifi: Some(WifiMarker::Connected),
        usb_present: true,
        bluetooth_present: true,
        unix_seconds: Some(1_767_225_600 + 9 * 3600 + 24 * 60),
    };
    let cases = [
        ("normal", normal),
        (
            "low-battery",
            StatusBar {
                battery: Some(BatteryMarker::Critical),
                usb_present: false,
                ..normal
            },
        ),
        (
            "offline",
            StatusBar {
                battery: Some(BatteryMarker::Level(1)),
                wifi: Some(WifiMarker::Disconnected),
                usb_present: false,
                bluetooth_present: false,
                ..normal
            },
        ),
        (
            "waiting",
            StatusBar {
                battery: None,
                wifi: Some(WifiMarker::Connecting),
                unix_seconds: None,
                ..normal
            },
        ),
        (
            "failed",
            StatusBar {
                wifi: Some(WifiMarker::Failed),
                ..normal
            },
        ),
    ];
    for (name, status) in cases {
        let mut frame = Frame::white();
        for (text, y) in [
            ("quasiquote0", 25),
            ("A quiet place for useful things.", 54),
            ("One-bit display / status preview", 125),
        ] {
            Text::new(
                text,
                Point::new(14, y),
                MonoTextStyle::new(&FONT_6X10, BinaryColor::Off),
            )
            .draw(&mut LandscapeFramebuffer(&mut frame))?;
        }
        for x in 14..260 {
            frame.set_pixel(x, 78, false);
        }
        status.overlay(&mut frame);
        fs::write(output.join(format!("{name}.bin")), frame.as_bytes())?;
        let mut file = fs::File::create(output.join(format!("{name}.pbm")))?;
        file.write_all(b"P4\n296 152\n")?;
        // PBM uses one for black; the device wire format uses one for white.
        let pixels: Vec<_> = frame.as_bytes().iter().map(|byte| !byte).collect();
        file.write_all(&pixels)?;
    }
    println!(
        "Rendered production-overlay previews in {}",
        output.display()
    );
    Ok(())
}
