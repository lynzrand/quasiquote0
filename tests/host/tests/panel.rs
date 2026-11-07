//! Exercise the actual UC8251D transport with virtual GPIO, SPI, and delays.
extern crate self as esp_idf_hal;
extern crate self as esp_idf_sys;

#[path = "support/panel_hal.rs"]
mod hal;
pub use hal::*;
pub use quasiquote0_host_tests::display::{status_window, Frame};
pub use quasiquote0_host_tests::{device, frame, refresh_policy, status_bar};
#[path = "../../../src/display/luts.rs"]
mod luts;
#[path = "../../../src/appliance/render.rs"]
mod render;
#[path = "../../../src/display/uc8251d.rs"]
mod uc8251d;
use core::marker::PhantomData;
use uc8251d::{Epd, EpdPins};

mod display {
    pub use super::uc8251d::Epd;
}
mod rtc_schedule {
    pub fn now_ms() -> u64 {
        super::PANEL.with_borrow(|panel| panel.elapsed_ms)
    }
}
mod wall_clock {
    pub fn unix_seconds() -> i64 {
        1_767_225_600
    }
}
mod power {
    pub struct Guard {
        pub timer_wake: bool,
    }
    pub fn guard() -> Guard {
        Guard { timer_wake: false }
    }
}

fn panel() -> Epd<'static> {
    PANEL.with_borrow_mut(|panel| *panel = Panel::default());
    Epd::new(
        spi::SPI2(PhantomData),
        gpio::Gpio10(PhantomData),
        gpio::Gpio7(PhantomData),
        EpdPins {
            busy: gpio::Gpio3(PhantomData),
            reset: gpio::Gpio4(PhantomData),
            data_command: gpio::Gpio5(PhantomData),
            chip_select: gpio::Gpio6(PhantomData),
            power: gpio::Gpio20(PhantomData),
        },
    )
    .unwrap()
}

#[test]
fn full_refresh_timeouts_power_off_and_retry_with_controller_initialization() {
    // Review found the full-refresh BUSY error bypassed cleanup. A retry
    // then skipped initialization and reused the failed controller state.
    for command in [0x04, 0x12, 0x02] {
        let mut display = panel();
        PANEL.with_borrow_mut(|panel| panel.busy_at = Some(command));
        assert_eq!(display.display(&Frame::white()), Err("EPD busy timeout"));
        PANEL.with_borrow(|panel| assert!(!panel.powered));

        PANEL.with_borrow_mut(|panel| panel.busy_at = None);
        assert_eq!(display.display(&Frame::white()), Ok(()));
        PANEL.with_borrow(|panel| {
            assert!(!panel.powered);
            assert_eq!(
                panel
                    .commands
                    .iter()
                    .filter(|&&command| command == 0x04)
                    .count(),
                2
            );
        });
    }
}

#[test]
fn partial_timeout_cold_initializes_the_full_refresh_fallback() {
    let mut display = panel();
    PANEL.with_borrow_mut(|panel| panel.busy_at = Some(0x12));
    assert_eq!(
        display.display_status_strip(&Frame::white()),
        Err("EPD busy timeout")
    );
    PANEL.with_borrow(|panel| assert!(!panel.powered));
    PANEL.with_borrow_mut(|panel| panel.busy_at = None);
    assert_eq!(display.display(&Frame::white()), Ok(()));
    PANEL.with_borrow(|panel| {
        assert!(!panel.powered);
        assert_eq!(
            panel
                .commands
                .iter()
                .filter(|&&command| command == 0x04)
                .count(),
            2
        );
    });
}

#[test]
fn retained_frame_errors_are_reported_without_painting_and_clear_after_repair() {
    use std::{fs, io::Write, sync::Mutex};
    // Review found load failures were silently discarded, so the status API
    // showed no error while the clock and content stopped refreshing.
    for corrupt in [false, true] {
        let root =
            std::env::temp_dir().join(format!("quote0-render-{}-{corrupt}", std::process::id()));
        let mut device = device::Device::open_at(root.clone()).unwrap();
        let source = frame::Frame::white();
        let mut writer = device.begin_frame_write(source.sha256()).unwrap();
        writer.write_all(source.as_bytes()).unwrap();
        device.commit_frame(writer.finish().unwrap()).unwrap();
        let path = root.join(format!("frames/{}.bin", source.sha256_hex()));
        if corrupt {
            fs::write(&path, frame::Frame::black().as_bytes()).unwrap();
        } else {
            fs::remove_file(&path).unwrap();
        }
        let device = Mutex::new(device);
        let mut renderer = render::Renderer::new(Some(panel()), Some(&device));
        renderer.refresh(&device, status_bar::StatusBar::default());
        let expected = if corrupt { "digest" } else { "missing" };
        assert!(device
            .lock()
            .unwrap()
            .display_error
            .as_deref()
            .unwrap()
            .contains(expected));
        assert_eq!(device.lock().unwrap().display_refreshes, 0);
        PANEL.with_borrow(|panel| assert!(panel.commands.is_empty()));

        fs::write(&path, source.as_bytes()).unwrap();
        PANEL.with_borrow_mut(|panel| panel.elapsed_ms += 5_000);
        renderer.refresh(&device, status_bar::StatusBar::default());
        assert!(device.lock().unwrap().display_error.is_none());
        assert_eq!(device.lock().unwrap().display_refreshes, 1);
        fs::remove_dir_all(root).unwrap();
    }
}
