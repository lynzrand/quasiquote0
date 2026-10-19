//! Quote/0 appliance firmware entry point.
use esp_idf_hal::peripherals::Peripherals;
use quasiquote0::display::{Epd, EpdPins};
use quasiquote0::refresh_policy::{RefreshKey, RefreshTracker};
use quasiquote0::status_bar::{BatteryMarker, StatusBar, WifiMarker};
use quasiquote0::usb_serial::UsbSerial;
use quasiquote0::{
    activity::OPERATIONS, app::App, battery, clock_face, link_patches, network::Network,
    pull_schedule::PullSchedule, rtc_schedule, wall_clock,
};
use std::sync::{atomic::Ordering, Arc, Mutex};
use std::thread;

fn main() {
    link_patches();
    let Peripherals {
        pins,
        spi2,
        modem,
        usb_serial,
        ..
    } = Peripherals::take().expect("peripherals already taken");
    // Establish physical recovery before optional panel and radio setup.
    let usb = UsbSerial::new(usb_serial, pins.gpio18, pins.gpio19).expect("USB setup failed");
    let app = App::mount();
    let _ = quasiquote0::power::arm_wake_sources(60);
    let bootstrap = app.clone();
    thread::Builder::new()
        .name("usb-setup".into())
        .stack_size(12 * 1024)
        .spawn(move || quasiquote0::bootstrap::serve(usb, bootstrap))
        .expect("USB setup task failed");
    let mut display = Epd::new(
        spi2,
        pins.gpio10,
        pins.gpio7,
        EpdPins {
            busy: pins.gpio3,
            reset: pins.gpio4,
            data_command: pins.gpio5,
            chip_select: pins.gpio6,
            power: pins.gpio20,
        },
    )
    .ok();
    // Confirm only after the owner loop has survived radio/server startup and
    // the panel has completed a refresh. A startup reset can still roll back.
    let mut boot_confirmed = false;
    let bluetooth = quasiquote0::bluetooth::Bluetooth::start().ok();
    let mut network = Network::new(modem);
    let mut server = None;
    let mut server_retry_at = 0;
    let mut schedule = rtc_schedule::load();
    let mut refreshes = RefreshTracker::default();
    let wifi_marker = Arc::new(Mutex::new(Some(WifiMarker::Disconnected)));
    if quasiquote0::power::guard().timer_wake {
        if let Some(device) = &app.device {
            let device = device.lock().unwrap();
            if !device.config().display.status_bar
                && device.state().displayed_status_bar == Some(false)
                && device.state().firmware_version.as_deref() == Some(env!("CARGO_PKG_VERSION"))
            {
                if let Ok(content) = device.desired_digest() {
                    if device.state().displayed_content_sha256.as_deref()
                        == Some(&quasiquote0::frame::digest_hex(&content))
                    {
                        refreshes.succeeded(RefreshKey::new(
                            content,
                            false,
                            wall_clock::unix_seconds(),
                        ));
                    }
                }
            }
        }
    }
    loop {
        let battery = battery::read();
        let usb = battery.usb_present;
        let config = app.config();
        if !usb {
            server.take();
        }
        if app.reboot_requested.load(Ordering::Acquire) {
            if let Some(_operation) = OPERATIONS.try_acquire() {
                esp_idf_hal::delay::FreeRtos::delay_ms(1_000);
                unsafe {
                    esp_idf_sys::esp_restart();
                }
            }
        }
        if !usb
            && battery
                .millivolts
                .is_some_and(|mv| mv < quasiquote0::power::CRITICAL_BATTERY_MV)
        {
            network.tick(&app, &config, false, false);
            rtc_schedule::save(&schedule);
            let _ = quasiquote0::power::try_deep_sleep(quasiquote0::power::CRITICAL_SLEEP_SECONDS);
            esp_idf_hal::delay::FreeRtos::delay_ms(1_000);
            continue;
        }
        let due = config.pull.enabled
            && schedule.is_due(
                rtc_schedule::now_ms(),
                app.pull_requested.load(Ordering::Acquire),
            );
        let link = network.tick(&app, &config, usb, due);
        if usb
            && app.network.lock().unwrap().connected
            && server.is_none()
            && rtc_schedule::now_ms() >= server_retry_at
        {
            match quasiquote0::management::start(app.clone()) {
                Ok(web) => server = Some(web),
                Err(error) => {
                    app.network.lock().unwrap().error = Some(format!("Browser service: {error}"));
                    server_retry_at = rtc_schedule::now_ms().saturating_add(30_000);
                }
            }
        }
        if due && link.failed {
            pull_failed(&app, &mut schedule, "Wi-Fi connection failed".into());
        } else if due && link.ready && !link.time_waiting {
            if let Some(_operation) = OPERATIONS.try_acquire() {
                // Consume the request before work so a later request is retained.
                app.pull_requested.store(false, Ordering::Release);
                let result = if config.pull.url.starts_with("https://")
                    && !clock_face::valid_time(wall_clock::unix_seconds())
                {
                    Err("NTP has not established time for HTTPS certificate validation".into())
                } else {
                    run_scheduled_pull(&app)
                };
                match result {
                    Ok(()) => {
                        schedule.succeeded(rtc_schedule::now_ms(), config.pull.interval_seconds)
                    }
                    Err(error) => pull_failed(&app, &mut schedule, error),
                }
                if !usb {
                    network.tick(&app, &config, false, false);
                }
            }
        }
        let connected = {
            let status = app.network.lock().unwrap();
            status.mode == "station" && status.connected
        };
        *wifi_marker.lock().unwrap() = Some(if connected {
            WifiMarker::Connected
        } else if schedule.consecutive_failures() > 0 {
            WifiMarker::Failed
        } else if link.busy {
            WifiMarker::Connecting
        } else {
            WifiMarker::Disconnected
        });
        if let Some(bluetooth) = &bluetooth {
            bluetooth.advertise(&quasiquote0::bthome::BthomeAdvertisement {
                battery_percent: battery.percent.unwrap_or_default() as u8,
                battery_mv: battery.millivolts.unwrap_or_default() as u16,
                battery_low: battery.percent.is_some_and(|percent| percent < 20),
                charging: usb,
                wifi_connected: connected,
                usb_present: usb,
            });
        }
        app.bluetooth_advertising.store(
            bluetooth.as_ref().is_some_and(|b| b.is_advertising()),
            Ordering::Relaxed,
        );
        if let Some(device) = &app.device {
            refresh_pending_frame(
                display.as_mut(),
                device,
                &wifi_marker,
                bluetooth.as_ref().is_some_and(|b| b.is_advertising()),
                &mut refreshes,
            );
        }
        if !boot_confirmed && quasiquote0::power::guard().uptime_ms >= 10_000 {
            if let Some(device) = &app.device {
                let mut device = device.lock().unwrap();
                if device.display_refreshes > 0
                    && device.display_error.is_none()
                    && app.recovery_error.is_none()
                {
                    boot_confirmed = device.confirm_ota().is_ok();
                }
            }
        }
        let mut seconds = if config.display.status_bar {
            clock_face::seconds_until_next_minute(wall_clock::unix_seconds())
        } else {
            quasiquote0::power::IDLE_SLEEP_SECONDS
        };
        if config.pull.enabled {
            seconds = seconds.min(schedule.seconds_until_due(rtc_schedule::now_ms()));
        }
        app.main_stack_free.store(
            unsafe { esp_idf_sys::uxTaskGetStackHighWaterMark(core::ptr::null_mut()) },
            Ordering::Relaxed,
        );
        rtc_schedule::save(&schedule);
        // Let connection and initial NTP finish before a battery pull can sleep.
        if !link.busy && !link.time_waiting {
            let _ = quasiquote0::power::try_deep_sleep(seconds);
        }
        esp_idf_hal::delay::FreeRtos::delay_ms(250);
    }
}
fn pull_failed(app: &App, schedule: &mut PullSchedule, error: String) {
    app.pull_requested.store(false, Ordering::Release);
    schedule.failed(rtc_schedule::now_ms());
    if let Some(device) = &app.device {
        device.lock().unwrap().record_pull_error(error);
    }
}

/// Recompose from the producer's retained frame for content/layout changes or
/// a new minute. A blank unconfigured device still gets the heartbeat strip.
fn refresh_pending_frame(
    mut display: Option<&mut Epd<'_>>,
    device: &Arc<Mutex<quasiquote0::device::Device>>,
    wifi_marker: &Arc<Mutex<Option<WifiMarker>>>,
    bluetooth_present: bool,
    refreshes: &mut RefreshTracker,
) {
    let Some(display) = display.as_deref_mut() else {
        return;
    };
    let seconds = wall_clock::unix_seconds();
    let now_ms = rtc_schedule::now_ms();
    let candidate = {
        let device = device.lock().unwrap();
        device.desired_digest().ok().and_then(|content| {
            let key = RefreshKey::new(content, device.config().display.status_bar, seconds);
            if !refreshes.is_due(key, now_ms) {
                return None;
            }
            match device.load_desired_frame() {
                Ok(None) if device.state().desired.is_some() => None,
                Ok(frame) => Some((frame.unwrap_or_default(), key)),
                Err(_) => None,
            }
        })
    };
    let Some((mut frame, key)) = candidate else {
        return;
    };
    if key.status_bar {
        let battery = battery::read();
        StatusBar {
            battery: if battery
                .millivolts
                .is_some_and(|mv| mv < quasiquote0::power::CRITICAL_BATTERY_MV)
            {
                Some(BatteryMarker::Critical)
            } else {
                StatusBar::from_battery_percent(battery.percent, false)
            },
            wifi: Some(
                wifi_marker
                    .lock()
                    .unwrap()
                    .unwrap_or(WifiMarker::Disconnected),
            ),
            usb_present: battery.usb_present,
            bluetooth_present,
            unix_seconds: Some(seconds),
        }
        .overlay(&mut frame);
    }
    let composed = frame.sha256();
    let projected = frame.project();
    let partial = cfg!(feature = "experimental-partial") && refreshes.can_refresh_status_only(key);
    let started = rtc_schedule::now_ms();
    let mut mode = if partial { "partial-window" } else { "full" };
    let mut partial_error = None;
    let transfer = if partial {
        match display.display_status_strip(&projected) {
            Ok(()) => Ok(()),
            Err(error) => {
                partial_error = Some(error.to_owned());
                mode = "full-fallback";
                display.display(&projected)
            }
        }
    } else {
        display.display(&projected)
    };
    let elapsed = rtc_schedule::now_ms().saturating_sub(started);
    let result = transfer.map_err(str::to_owned).and_then(|()| {
        device
            .lock()
            .unwrap()
            .mark_displayed(
                &key.content,
                &composed,
                env!("CARGO_PKG_VERSION"),
                seconds,
                key.status_bar,
            )
            .map_err(|error| error.to_string())
    });
    match result {
        Ok(()) => {
            if mode == "partial-window" {
                refreshes.partial_succeeded(key);
            } else {
                refreshes.succeeded(key);
            }
            let mut device = device.lock().unwrap();
            device.display_refresh_mode = mode;
            device.display_refresh_ms = elapsed;
            device.partial_refresh_error = partial_error;
        }
        Err(error) => {
            refreshes.failed(rtc_schedule::now_ms());
            device.lock().unwrap().display_error = Some(error);
        }
    }
}

/// The caller owns the publication lease and an already-connected station.
fn run_scheduled_pull(app: &App) -> Result<(), String> {
    let device = app.device.as_ref().ok_or("Storage recovery required")?;
    let (config, validators) = device.lock().unwrap().pull_snapshot();
    device.lock().unwrap().pull_attempts += 1;
    let mut writer = device
        .lock()
        .unwrap()
        .begin_unannounced_frame_write()
        .map_err(|e| e.to_string())?;
    let response = quasiquote0::pull::fetch(&config.pull, &validators, &mut writer)?;
    match response.status {
        200 => {
            let digest = writer.finish().map_err(|e| e.to_string())?;
            if response
                .announced_sha256
                .as_deref()
                .is_some_and(|value| value != quasiquote0::frame::digest_hex(&digest))
            {
                let _ = device.lock().unwrap().discard_unreferenced_frames();
                return Err("X-Frame-SHA256 does not match response body".into());
            }
            device
                .lock()
                .unwrap()
                .commit_pulled_frame(
                    digest,
                    quasiquote0::store::PullValidators {
                        etag: response.etag,
                        last_modified: response.last_modified,
                    },
                )
                .map_err(|e| e.to_string())
        }
        204 | 304 => Ok(()),
        status => Err(format!("HTTP pull returned status {status}")),
    }
}
