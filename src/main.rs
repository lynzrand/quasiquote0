//! Quote/0 appliance firmware entry point.

use esp_idf_hal::peripherals::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use quasiquote0::display::{Epd, EpdPins};
use quasiquote0::frame::Projection;
use quasiquote0::jsonrpc;
use quasiquote0::pull_schedule::PullSchedule;
use quasiquote0::rpc::ControlRpc;
use quasiquote0::status_bar::{BatteryMarker, StatusBar, WifiMarker};
use quasiquote0::usb_serial::UsbSerial;
use quasiquote0::wifi::WifiStation;
use quasiquote0::yamux::{Mode, Yamux};
use quasiquote0::{battery, link_patches};
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
    // Bluetooth provisioning/advertising is intentionally outside the first
    // release. Do not start NimBLE before the USB recovery/control service.
    run_display_loop(display.as_mut(), device, wifi, wifi_marker, false);
}

fn run_display_loop(
    mut display: Option<&mut Epd<'_>>,
    device: Arc<Mutex<quasiquote0::device::Device>>,
    mut wifi: Option<WifiStation<'_>>,
    wifi_marker: Arc<Mutex<Option<WifiMarker>>>,
    bluetooth_present: bool,
) -> ! {
    let mut pull_schedule = PullSchedule::default();
    loop {
        let now_ms = (unsafe { esp_idf_sys::esp_timer_get_time() } / 1_000) as u64;
        if let Some(wifi) = wifi.as_mut() {
            run_scheduled_pull(&device, wifi, &wifi_marker, now_ms, &mut pull_schedule);
        }
        let candidate = {
            let device = device.lock().unwrap();
            match device.state().desired.as_ref() {
                Some(desired)
                    if device.state().displayed_content_sha256.as_deref()
                        != Some(&desired.sha256) =>
                {
                    device
                        .load_desired_frame()
                        .ok()
                        .flatten()
                        .map(|frame| (frame, device.config().display.status_bar))
                }
                _ => None,
            }
        };
        if let (Some(display), Some((mut frame, status_bar))) = (display.as_deref_mut(), candidate)
        {
            let content = frame.sha256();
            if status_bar {
                let battery = battery::read();
                StatusBar {
                    battery: StatusBar::from_battery_percent(battery.percent, false)
                        .or(Some(BatteryMarker::Critical)),
                    wifi: *wifi_marker.lock().unwrap(),
                    usb_present: battery.usb_present,
                    bluetooth_present,
                }
                .overlay(&mut frame);
            }
            let composed = frame.sha256();
            let projected = frame.project(Projection::Clockwise);
            if display.display(&projected).is_ok() {
                let _ = device.lock().unwrap().mark_displayed(
                    &content,
                    &composed,
                    env!("CARGO_PKG_VERSION"),
                );
            }
        }
        esp_idf_hal::delay::FreeRtos::delay_ms(1_000);
    }
}

fn run_scheduled_pull(
    device: &Arc<Mutex<quasiquote0::device::Device>>,
    wifi: &mut WifiStation<'_>,
    wifi_marker: &Arc<Mutex<Option<WifiMarker>>>,
    now_ms: u64,
    schedule: &mut PullSchedule,
) {
    let (config, validators) = device.lock().unwrap().pull_snapshot();
    let requested =
        quasiquote0::rpc::PULL_REQUESTED.swap(false, std::sync::atomic::Ordering::SeqCst);
    if !config.pull.enabled || !schedule.is_due(now_ms, requested) {
        return;
    }
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
            schedule.succeeded(now_ms, config.pull.interval_seconds);
            *wifi_marker.lock().unwrap() = Some(WifiMarker::Connected);
        }
        Err(error) => {
            schedule.failed(now_ms);
            *wifi_marker.lock().unwrap() = Some(WifiMarker::Failed);
            let _ = device.lock().unwrap().record_pull_error(error);
        }
    }
}
