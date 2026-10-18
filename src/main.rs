//! Quote/0 appliance firmware entry point.

use esp_idf_hal::peripherals::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use quasiquote0::display::{Epd, EpdPins};
use quasiquote0::frame::Projection;
use quasiquote0::jsonrpc;
use quasiquote0::pull_schedule::PullSchedule;
use quasiquote0::refresh_policy::{RefreshKey, RefreshTracker};
use quasiquote0::rpc::ControlRpc;
use quasiquote0::status_bar::{BatteryMarker, StatusBar, WifiMarker};
use quasiquote0::usb_serial::UsbSerial;
use quasiquote0::wifi::WifiStation;
use quasiquote0::yamux::{Mode, Yamux};
use quasiquote0::{battery, link_patches};
use quasiquote0::{clock_face, rtc_schedule, wall_clock};
use std::sync::{Arc, Mutex};
use std::thread;

fn main() {
    link_patches();

    let peripherals = Peripherals::take().expect("peripherals already taken");
    let Peripherals {
        pins,
        spi2,
        modem,
        usb_serial,
        ..
    } = peripherals;
    // USB must become usable before optional display/radio setup. This keeps
    // recovery and configuration available if either later subsystem fails.
    let usb = UsbSerial::new(usb_serial, pins.gpio18, pins.gpio19).expect("USB setup failed");
    let yamux = Yamux::start(usb, Mode::Server).expect("Yamux driver failed");
    let mut rpc = ControlRpc::mount(yamux.clone());
    let _ = quasiquote0::power::arm_wake_sources(60);
    let Some(device) = rpc.device_handle() else {
        jsonrpc::serve(yamux.control_stream(), &mut rpc).expect("recovery JSON-RPC failed");
        return;
    };
    let wifi_marker = Arc::new(Mutex::new(None));
    let control = yamux.control_stream();
    thread::Builder::new()
        .name("usb-control".into())
        .stack_size(8 * 1024)
        .spawn(move || {
            jsonrpc::serve(control, &mut rpc).expect("stream-0 JSON-RPC failed");
        })
        .expect("USB control task failed");

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
    let wifi = EspSystemEventLoop::take()
        .ok()
        .and_then(|event_loop| {
            EspDefaultNvsPartition::take()
                .ok()
                .map(|nvs| (event_loop, nvs))
        })
        .and_then(|(event_loop, nvs)| WifiStation::new(modem, event_loop, Some(nvs)).ok());
    // The BLE beacon starts after the USB control path is already alive so a
    // failing radio stack can never block recovery or configuration.
    let bluetooth = quasiquote0::bluetooth::Bluetooth::start().ok();
    run_appliance(
        display.as_mut(),
        device,
        wifi,
        wifi_marker,
        bluetooth.as_ref(),
    );
}

/// Minute wakes update the entire status strip. Pull deadlines use a separate
/// RTC counter and survive deep sleep, so a clock wake does not start Wi-Fi.
fn run_appliance(
    mut display: Option<&mut Epd<'_>>,
    device: Arc<Mutex<quasiquote0::device::Device>>,
    mut wifi: Option<WifiStation<'_>>,
    wifi_marker: Arc<Mutex<Option<WifiMarker>>>,
    bluetooth: Option<&quasiquote0::bluetooth::Bluetooth>,
) -> ! {
    let mut pull_schedule = rtc_schedule::load();
    let mut refreshes = RefreshTracker::default();
    // A bar-disabled timer wake retains unchanged content without a panel
    // refresh. Other boots re-establish a trustworthy visible image.
    if quasiquote0::power::guard().timer_wake {
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
    *wifi_marker.lock().unwrap() = Some(if pull_schedule.consecutive_failures() > 0 {
        WifiMarker::Failed
    } else {
        WifiMarker::Disconnected
    });
    loop {
        let battery = battery::read();
        // Critical-cell protection wins over the heartbeat and recovery window.
        // USB power still permits operation while the cell is charging.
        if !battery.usb_present
            && battery
                .millivolts
                .is_some_and(|mv| mv < quasiquote0::power::CRITICAL_BATTERY_MV)
        {
            rtc_schedule::save(&pull_schedule);
            let _ = quasiquote0::power::try_deep_sleep(quasiquote0::power::CRITICAL_SLEEP_SECONDS);
            esp_idf_hal::delay::FreeRtos::delay_ms(1_000);
            continue;
        }

        if let Some(bluetooth) = bluetooth {
            bluetooth.advertise(&quasiquote0::bluetooth::BthomeAdvertisement {
                battery_percent: battery.percent.unwrap_or_default() as u8,
                battery_mv: battery.millivolts.unwrap_or_default() as u16,
                battery_low: battery.percent.is_some_and(|percent| percent < 20),
                charging: battery.usb_present,
                wifi_connected: matches!(*wifi_marker.lock().unwrap(), Some(WifiMarker::Connected)),
                usb_present: battery.usb_present,
            });
        }
        if let Some(wifi) = wifi.as_mut() {
            run_scheduled_pull(&device, wifi, &wifi_marker, &mut pull_schedule);
        }
        refresh_pending_frame(
            display.as_deref_mut(),
            &device,
            &wifi_marker,
            bluetooth.is_some_and(|b| b.is_advertising()),
            &mut refreshes,
        );

        let config = device.lock().unwrap().config().clone();
        let mut sleep_seconds = if config.display.status_bar {
            clock_face::seconds_until_next_minute(wall_clock::unix_seconds())
        } else {
            quasiquote0::power::IDLE_SLEEP_SECONDS
        };
        if config.pull.enabled && wifi.is_some() {
            sleep_seconds =
                sleep_seconds.min(pull_schedule.seconds_until_due(rtc_schedule::now_ms()));
        }
        rtc_schedule::save(&pull_schedule);
        let _ = quasiquote0::power::try_deep_sleep(sleep_seconds);
        esp_idf_hal::delay::FreeRtos::delay_ms(250);
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
    let result = display
        .display(&frame.project(Projection::Clockwise))
        .map_err(str::to_owned)
        .and_then(|()| {
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
        Ok(()) => refreshes.succeeded(key),
        Err(error) => {
            refreshes.failed(rtc_schedule::now_ms());
            device.lock().unwrap().display_error = Some(error);
        }
    }
}

fn run_scheduled_pull(
    device: &Arc<Mutex<quasiquote0::device::Device>>,
    wifi: &mut WifiStation<'_>,
    wifi_marker: &Arc<Mutex<Option<WifiMarker>>>,
    schedule: &mut PullSchedule,
) {
    let (config, validators) = device.lock().unwrap().pull_snapshot();
    let now_ms = rtc_schedule::now_ms();
    let requested =
        quasiquote0::rpc::PULL_REQUESTED.swap(false, std::sync::atomic::Ordering::SeqCst);
    if !config.pull.enabled || !schedule.is_due(now_ms, requested) {
        return;
    }
    device.lock().unwrap().pull_attempts += 1;
    *wifi_marker.lock().unwrap() = Some(WifiMarker::Connecting);
    let result = (|| -> Result<(), String> {
        wifi.configure(&config.wifi.ssid, &config.wifi.password)?;
        let _ = wifi.start();
        let _ = wifi.connect();
        esp_idf_hal::delay::FreeRtos::delay_ms(5_000);
        if !wifi.is_up().map_err(|error| error.to_string())? {
            return Err("Wi-Fi did not obtain an IP address".to_owned());
        }
        let mut writer = device
            .lock()
            .unwrap()
            .begin_unannounced_frame_write()
            .map_err(|error| error.to_string())?;
        let response = quasiquote0::pull::fetch(&config.pull, &validators, &mut writer)?;
        match response.status {
            200 => {
                let digest = writer.finish().map_err(|error| error.to_string())?;
                if response
                    .announced_sha256
                    .as_deref()
                    .is_some_and(|value| value != quasiquote0::frame::digest_hex(&digest))
                {
                    let _ = device.lock().unwrap().discard_unreferenced_frames();
                    return Err("X-Frame-SHA256 does not match response body".to_owned());
                }
                device
                    .lock()
                    .unwrap()
                    .commit_pulled_frame(
                        digest,
                        quasiquote0::store::FrameMeta {
                            generation: response.etag.clone().unwrap_or_default(),
                            source: "http-pull".to_owned(),
                            accepted_at_ms: now_ms,
                        },
                        response.etag,
                        response.last_modified,
                        now_ms,
                    )
                    .map_err(|error| error.to_string())
            }
            204 | 304 => Ok(()),
            status => Err(format!("HTTP pull returned status {status}")),
        }
    })();
    match result {
        Ok(()) => {
            schedule.succeeded(rtc_schedule::now_ms(), config.pull.interval_seconds);
            // The station is stopped below; the next status snapshot must not
            // report an association that no longer exists.
            *wifi_marker.lock().unwrap() = Some(WifiMarker::Disconnected);
        }
        Err(error) => {
            schedule.failed(rtc_schedule::now_ms());
            *wifi_marker.lock().unwrap() = Some(WifiMarker::Failed);
            let _ = device.lock().unwrap().record_pull_error(error);
        }
    }
    // Radio time is over until the next due attempt; the next pull brings
    // the station back up. This matters most on battery, before deep sleep.
    let _ = wifi.stop();
}
