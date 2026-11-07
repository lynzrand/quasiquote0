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
mod rtc_display {
    pub fn load() -> Option<[u8; 32]> {
        super::PANEL.with_borrow(|panel| {
            if panel.deep_sleep_wake {
                panel.retained_composed
            } else {
                None
            }
        })
    }
    pub fn save(digest: Option<[u8; 32]>) {
        super::PANEL.with_borrow_mut(|panel| panel.retained_composed = digest);
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
        super::PANEL.with_borrow(|panel| Guard {
            timer_wake: panel.deep_sleep_wake,
        })
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

fn full_mode() -> &'static str {
    if cfg!(feature = "experimental-fast") {
        "fast-full"
    } else {
        "full"
    }
}

#[test]
fn identical_composed_pixels_skip_spi_within_a_boot_and_after_deep_sleep() {
    use std::{fs, io::Write, sync::Mutex};
    let root = std::env::temp_dir().join(format!("quote0-unchanged-{}", std::process::id()));
    let device = Mutex::new(device::Device::open_at(root.clone()).unwrap());
    let mut renderer = render::Renderer::new(Some(panel()), Some(&device));
    let status = status_bar::StatusBar {
        unix_seconds: Some(120),
        ..Default::default()
    };
    renderer.refresh(&device, status);
    assert_eq!(device.lock().unwrap().display_refreshes, 1);
    let retained = PANEL.with_borrow(|panel| panel.retained_composed);
    assert!(retained.is_some());
    PANEL.with_borrow_mut(|panel| {
        panel.commands.clear();
        panel.payloads.clear();
    });

    // A new minute before NTP still renders the same --:-- glyphs.
    renderer.refresh(
        &device,
        status_bar::StatusBar {
            unix_seconds: Some(180),
            ..status
        },
    );
    assert_eq!(device.lock().unwrap().display_refresh_mode, "unchanged");
    assert_eq!(device.lock().unwrap().display_refreshes, 1);
    PANEL.with_borrow(|panel| assert!(panel.commands.is_empty()));

    // The source digest changes only in reserved columns overwritten by the
    // overlay. Compare composed pixels rather than source metadata alone.
    let mut source = frame::Frame::white();
    source.set_pixel(frame::WIDTH - 1, 40, false);
    {
        let mut device = device.lock().unwrap();
        let mut writer = device.begin_frame_write(source.sha256()).unwrap();
        writer.write_all(source.as_bytes()).unwrap();
        device.commit_frame(writer.finish().unwrap()).unwrap();
    }
    renderer.refresh(&device, status);
    assert_eq!(device.lock().unwrap().display_refreshes, 1);
    PANEL.with_borrow(|panel| assert!(panel.commands.is_empty()));

    let display = panel(); // new controller object, only RTC pixels retained
    PANEL.with_borrow_mut(|panel| {
        panel.deep_sleep_wake = true;
        panel.retained_composed = retained;
    });
    let mut renderer = render::Renderer::new(Some(display), Some(&device));
    renderer.refresh(&device, status);
    assert_eq!(device.lock().unwrap().display_refresh_mode, "unchanged");
    assert_eq!(device.lock().unwrap().display_refresh_ms, 0);
    assert_eq!(device.lock().unwrap().display_refreshes, 1);
    PANEL.with_borrow(|panel| assert!(panel.commands.is_empty()));

    // Skipping on wake has no previous-frame buffer. The first actual pixel
    // change must establish a full baseline, not attempt a differential.
    renderer.refresh(
        &device,
        status_bar::StatusBar {
            unix_seconds: Some(1_767_225_600),
            ..status
        },
    );
    assert_eq!(device.lock().unwrap().display_refresh_mode, full_mode());
    assert_eq!(device.lock().unwrap().display_refreshes, 2);
    assert!(device.lock().unwrap().partial_refresh_error.is_none());

    // A failed update may have changed pixels. Never use its old hash to
    // skip repainting the original target, in RAM or after another wake.
    let failed = frame::Frame::black();
    {
        let mut device = device.lock().unwrap();
        let mut writer = device.begin_frame_write(failed.sha256()).unwrap();
        writer.write_all(failed.as_bytes()).unwrap();
        device.commit_frame(writer.finish().unwrap()).unwrap();
    }
    let valid_status = status_bar::StatusBar {
        unix_seconds: Some(1_767_225_600),
        ..status
    };
    PANEL.with_borrow_mut(|panel| panel.busy_at = Some(0x12));
    renderer.refresh(&device, valid_status);
    assert!(device.lock().unwrap().display_error.is_some());
    PANEL.with_borrow_mut(|panel| {
        assert!(panel.retained_composed.is_none());
        panel.busy_at = None;
        panel.elapsed_ms += 5_000;
    });
    {
        let mut device = device.lock().unwrap();
        let mut writer = device.begin_frame_write(source.sha256()).unwrap();
        writer.write_all(source.as_bytes()).unwrap();
        device.commit_frame(writer.finish().unwrap()).unwrap();
    }
    renderer.refresh(&device, valid_status);
    assert_eq!(device.lock().unwrap().display_refresh_mode, full_mode());
    assert_eq!(device.lock().unwrap().display_refreshes, 3);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(feature = "experimental-fast")]
#[test]
fn fast_refreshes_cold_initialize_and_full_writes_use_short_destination_luts() {
    let mut display = panel();
    display.display(&Frame::white()).unwrap();
    PANEL.with_borrow_mut(|panel| {
        assert!(!panel.powered);
        assert!(panel.commands.contains(&0x02));
        assert!(panel.commands.contains(&0x07));
        panel.commands.clear();
        panel.payloads.clear();
    });
    display
        .display_differential(&Frame::black(), false)
        .unwrap();
    PANEL.with_borrow(|panel| {
        assert!(!panel.powered);
        assert!(panel.commands.contains(&0x04));
        assert!(panel.commands.contains(&0x02));
        assert!(panel.commands.contains(&0x00));
        assert!(panel.commands.contains(&0x07));
        for (command, expected) in (0x20..=0x24).zip(luts::differential()) {
            assert_eq!(last_payload(panel, command), expected);
        }
    });
    display.display(&Frame::white()).unwrap();
    PANEL.with_borrow(|panel| {
        for (command, expected) in (0x20..=0x24).zip(luts::full()) {
            assert_eq!(last_payload(panel, command), expected);
        }
        // Unchanged pixels are driven to their destination in a full baseline.
        assert_eq!(last_payload(panel, 0x21)[1], 0x80);
        assert_eq!(last_payload(panel, 0x24)[1], 0x40);
    });
    display.recover(&Frame::white()).unwrap();
    PANEL.with_borrow(|panel| {
        for (command, expected) in (0x20..=0x24).zip(luts::stock()) {
            assert_eq!(last_payload(panel, command), expected);
        }
    });
}

#[cfg(feature = "experimental-fast")]
#[test]
fn first_full_write_uses_short_destination_pulses_for_both_colors_without_a_baseline() {
    for frame in [Frame::white(), Frame::black()] {
        let mut display = panel();
        display.display(&frame).unwrap();
        PANEL.with_borrow(|panel| {
            assert!(!panel.powered);
            assert_eq!(last_payload(panel, 0x10).len(), frame.as_bytes().len());
            assert!(last_payload(panel, 0x10).iter().all(|&byte| byte == 0));
            assert_eq!(last_payload(panel, 0x13), frame.as_bytes());
            for (command, packed) in [(0x21, 0x80), (0x22, 0x80), (0x23, 0x40), (0x24, 0x40)] {
                let table = last_payload(panel, command);
                assert_eq!(&table[..8], &[1, packed, 48, 1, 1, 1, 1, 1]);
                assert!(table[8..].iter().all(|&byte| byte == 0));
            }
        });
    }
}

#[cfg(feature = "experimental-fast")]
#[test]
fn failed_short_startup_recovers_with_stock_conditioning_and_acknowledges_once() {
    use std::{fs, sync::Mutex};
    let root = std::env::temp_dir().join(format!("quote0-short-startup-{}", std::process::id()));
    let device = Mutex::new(device::Device::open_at(root.clone()).unwrap());
    let display = panel();
    PANEL.with_borrow_mut(|panel| panel.spi_error_once = Some(0x12));
    let mut renderer = render::Renderer::new(Some(display), Some(&device));
    renderer.refresh(&device, status_bar::StatusBar::default());
    let device = device.lock().unwrap();
    assert!(device.display_error.is_none());
    assert_eq!(device.display_refresh_mode, "full-fallback");
    assert_eq!(device.display_refreshes, 1);
    assert_eq!(
        device.partial_refresh_error.as_deref(),
        Some("EPD SPI transfer failed")
    );
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
        for (command, expected) in (0x20..=0x24).zip(luts::stock()) {
            assert_eq!(last_payload(panel, command), expected);
        }
        assert!(panel.retained_composed.is_some());
    });
    fs::remove_dir_all(root).unwrap();
}

#[cfg(feature = "experimental-fast")]
#[test]
fn fast_timeouts_invalidate_the_baseline_and_cold_initialize_recovery() {
    for command in [0x04, 0x12, 0x02] {
        let mut display = panel();
        display.display(&Frame::white()).unwrap();
        PANEL.with_borrow_mut(|panel| panel.busy_at = Some(command));
        assert_eq!(
            display.display_differential(&Frame::black(), false),
            Err("EPD busy timeout")
        );
        PANEL.with_borrow_mut(|panel| {
            assert!(!panel.powered);
            panel.busy_at = None;
            panel.commands.clear();
            panel.payloads.clear();
        });
        assert_eq!(
            display.display_differential(&Frame::white(), false),
            Err("EPD partial baseline missing")
        );
        display.display(&Frame::white()).unwrap();
        PANEL.with_borrow(|panel| assert!(panel.commands.contains(&0x00)));
    }
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
    display.display(&Frame::white()).unwrap();
    PANEL.with_borrow_mut(|panel| {
        panel.commands.clear();
        panel.payloads.clear();
    });
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

#[cfg(feature = "experimental-differential")]
fn last_payload(panel: &Panel, command: u8) -> &[u8] {
    &panel
        .payloads
        .iter()
        .rev()
        .find(|(sent, _)| *sent == command)
        .unwrap()
        .1
}

#[cfg(all(
    feature = "experimental-differential",
    not(feature = "experimental-fast")
))]
#[test]
fn differential_windows_keep_changed_pixel_waveforms_and_upload_the_real_baseline() {
    // Each adjacent pair exercises BB, BW, WB, or WW. An all-zero or all-new
    // DTM1 plane would silently select the wrong waveform for some pixels.
    let mut display = panel();
    let mut previous = Frame::white();
    let mut next = Frame::white();
    for x in 0..8 {
        previous.set_pixel(x, 0, 0x33 & (0x80 >> x) != 0);
        next.set_pixel(x, 0, 0x55 & (0x80 >> x) != 0);
    }
    display.display(&previous).unwrap();
    display.display_status_strip(&next).unwrap();
    PANEL.with_borrow(|panel| {
        assert!(!panel.powered);
        assert_eq!(last_payload(panel, 0x10), status_window::plane(&previous));
        assert_eq!(last_payload(panel, 0x13), status_window::plane(&next));
        assert_eq!(last_payload(panel, 0x90), status_window::COMMAND);
        for (command, stock) in [
            (0x20, luts::lut_20()),
            (0x22, luts::lut_22()),
            (0x23, luts::lut_23()),
        ] {
            assert_eq!(last_payload(panel, command), stock);
        }
        for (command, stock) in [(0x21, luts::lut_21()), (0x24, luts::lut_24())] {
            let unchanged = last_payload(panel, command);
            assert_eq!(unchanged.len(), stock.len());
            for (index, (&byte, &original)) in unchanged.iter().zip(stock).enumerate() {
                assert_eq!(byte, if index % 8 == 1 { 0 } else { original });
            }
        }
    });

    display.display_status_strip(&previous).unwrap();
    PANEL.with_borrow(|panel| {
        assert_eq!(last_payload(panel, 0x10), status_window::plane(&next));
        assert_eq!(last_payload(panel, 0x13), status_window::plane(&previous));
    });

    // A later full cleanup must reload the original WW/BB waveforms.
    display.display(&next).unwrap();
    PANEL.with_borrow(|panel| {
        assert_eq!(last_payload(panel, 0x21), luts::lut_21());
        assert_eq!(last_payload(panel, 0x24), luts::lut_24());
        assert!(last_payload(panel, 0x10).iter().all(|&byte| byte == 0));
    });
}

#[cfg(feature = "experimental-differential")]
#[test]
fn differential_windows_require_a_successful_baseline_and_forget_it_after_a_timeout() {
    for status_only in [true, false] {
        let mut display = panel();
        assert_eq!(
            display.display_differential(&Frame::white(), status_only),
            Err("EPD partial baseline missing")
        );
        PANEL.with_borrow(|panel| assert!(panel.commands.is_empty()));

        display.display(&Frame::white()).unwrap();
        PANEL.with_borrow_mut(|panel| panel.busy_at = Some(0x12));
        assert_eq!(
            display.display_differential(&Frame::black(), status_only),
            Err("EPD busy timeout")
        );
        PANEL.with_borrow_mut(|panel| {
            assert!(!panel.powered);
            panel.busy_at = None;
            panel.commands.clear();
            panel.payloads.clear();
        });
        assert_eq!(
            display.display_differential(&Frame::white(), status_only),
            Err("EPD partial baseline missing")
        );
        PANEL.with_borrow(|panel| assert!(panel.commands.is_empty()));

        display.display(&Frame::black()).unwrap();
        display
            .display_differential(&Frame::white(), status_only)
            .unwrap();
        PANEL.with_borrow(|panel| {
            assert!(last_payload(panel, 0x10).iter().all(|&byte| byte == 0));
        });
    }
}

#[cfg(feature = "experimental-differential")]
#[test]
fn main_image_differential_uses_the_full_baseline_and_preserves_body_after_strip_updates() {
    let mut display = panel();
    let mut baseline = Frame::white();
    baseline.set_pixel(10, 200, false);
    display.display(&baseline).unwrap();

    let mut strip = baseline.clone();
    strip.set_pixel(3, 0, false);
    strip.set_pixel(10, 200, true); // outside the window: must not alter the baseline
    display.display_status_strip(&strip).unwrap();
    let mut displayed = baseline.clone();
    displayed.set_pixel(3, 0, false);

    let mut next = baseline.clone();
    next.set_pixel(10, 200, true);
    next.set_pixel(151, 295, false); // exercise the final transfer's tail
    PANEL.with_borrow_mut(|panel| {
        panel.commands.clear();
        panel.payloads.clear();
    });
    display.display_differential(&next, false).unwrap();
    PANEL.with_borrow(|panel| {
        assert!(!panel.powered);
        assert_eq!(last_payload(panel, 0x10), displayed.as_bytes());
        assert_eq!(last_payload(panel, 0x13), next.as_bytes());
        assert!(!panel.commands.contains(&0x91));
        assert!(!panel.commands.contains(&0x90));
        assert_eq!(last_payload(panel, 0x22), luts::differential()[2]);
        assert_eq!(last_payload(panel, 0x23), luts::differential()[3]);
        assert_eq!(last_payload(panel, 0x21), &luts::lut_unchanged()[..56]);
        assert_eq!(last_payload(panel, 0x24), luts::lut_unchanged());
    });
    display.display_differential(&baseline, false).unwrap();
    PANEL.with_borrow(|panel| assert_eq!(last_payload(panel, 0x10), next.as_bytes()));
}

#[cfg(feature = "experimental-differential")]
#[test]
fn uploaded_main_images_use_differential_refresh_with_periodic_full_baselines() {
    use std::{fs, io::Write, sync::Mutex};
    let root =
        std::env::temp_dir().join(format!("quote0-main-differential-{}", std::process::id()));
    let device = Mutex::new(device::Device::open_at(root.clone()).unwrap());
    let mut renderer = render::Renderer::new(Some(panel()), Some(&device));
    let status = status_bar::StatusBar {
        unix_seconds: Some(1_767_225_600),
        ..Default::default()
    };
    let mut previous = Frame::white();
    for image in 0..=5 {
        let mut source = frame::Frame::white();
        source.set_pixel(20 + image, 20, false);
        {
            let mut device = device.lock().unwrap();
            let mut writer = device.begin_frame_write(source.sha256()).unwrap();
            writer.write_all(source.as_bytes()).unwrap();
            device.commit_frame(writer.finish().unwrap()).unwrap();
        }
        renderer.refresh(&device, status);
        {
            let device = device.lock().unwrap();
            assert!(device.display_error.is_none());
            assert!(device.partial_refresh_error.is_none());
            assert_eq!(device.display_refreshes, (image + 1) as u32);
            assert_eq!(
                device.display_refresh_mode,
                if image == 0 || image == 5 {
                    full_mode()
                } else {
                    if cfg!(feature = "experimental-fast") {
                        "fast-differential"
                    } else {
                        "differential"
                    }
                }
            );
        }
        if (1..=4).contains(&image) {
            PANEL.with_borrow(|panel| assert_eq!(last_payload(panel, 0x10), previous.as_bytes()));
        }
        status.overlay(&mut source);
        previous = source.project();
    }
    fs::remove_dir_all(root).unwrap();
}

#[cfg(feature = "experimental-differential")]
#[test]
fn idle_clock_updates_remain_windowed_beyond_the_former_full_refresh_budget() {
    use std::{fs, sync::Mutex};
    let root = std::env::temp_dir().join(format!("quote0-differential-{}", std::process::id()));
    let device = Mutex::new(device::Device::open_at(root.clone()).unwrap());
    let mut renderer = render::Renderer::new(Some(panel()), Some(&device));
    for minute in 0..=12 {
        renderer.refresh(
            &device,
            status_bar::StatusBar {
                unix_seconds: Some(1_767_225_600 + minute * 60),
                ..Default::default()
            },
        );
        let device = device.lock().unwrap();
        assert!(device.display_error.is_none());
        assert!(device.partial_refresh_error.is_none());
        assert_eq!(device.display_refreshes, (minute + 1) as u32);
        assert_eq!(
            device.display_refresh_mode,
            if minute == 0 {
                full_mode()
            } else {
                if cfg!(feature = "experimental-fast") {
                    "fast-window"
                } else {
                    "partial-differential"
                }
            }
        );
        if minute > 0 {
            PANEL.with_borrow(|panel| {
                assert_eq!(last_payload(panel, 0x10).len(), status_window::BYTES);
                assert_eq!(last_payload(panel, 0x13).len(), status_window::BYTES);
            });
        }
    }
    fs::remove_dir_all(root).unwrap();
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
