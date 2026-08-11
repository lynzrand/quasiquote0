//! Quote/0 appliance firmware entry point.

use esp_idf_hal::peripherals::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use quasiquote0::bluetooth::Bluetooth;
use quasiquote0::display::{Epd, EpdPins};
use quasiquote0::frame::Projection;
use quasiquote0::jsonrpc;
use quasiquote0::rpc::ControlRpc;
use quasiquote0::status_bar::{BatteryMarker, StatusBar};
use quasiquote0::usb_serial::UsbSerial;
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
        modem: _,
        usb_serial,
        ..
    } = peripherals;
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
    .expect("EPD setup failed");

    // Reserve the IDF system singletons now; the pull worker owns actual
    // station setup once a persistent Wi-Fi configuration is present.
    let _event_loop = EspSystemEventLoop::take();
    let _nvs = EspDefaultNvsPartition::take();
    let bluetooth = Bluetooth::start().ok();

    let usb = UsbSerial::new(usb_serial, pins.gpio18, pins.gpio19).expect("USB setup failed");
    let yamux = Yamux::start(usb, Mode::Server).expect("Yamux driver failed");
    let mut rpc = ControlRpc::mount(yamux.clone());
    let Some(device) = rpc.device_handle() else {
        jsonrpc::serve(yamux.control_stream(), &mut rpc).expect("recovery JSON-RPC failed");
        return;
    };
    let control = yamux.control_stream();
    thread::Builder::new()
        .name("usb-control".into())
        .stack_size(8 * 1024)
        .spawn(move || {
            jsonrpc::serve(control, &mut rpc).expect("stream-0 JSON-RPC failed");
        })
        .expect("USB control task failed");

    run_display_loop(
        &mut display,
        device,
        bluetooth.as_ref().is_some_and(Bluetooth::is_ready),
    );
}

fn run_display_loop(
    display: &mut Epd<'_>,
    device: Arc<Mutex<quasiquote0::device::Device>>,
    bluetooth_present: bool,
) -> ! {
    loop {
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
        if let Some((mut frame, status_bar)) = candidate {
            let content = frame.sha256();
            if status_bar {
                let battery = battery::read();
                StatusBar {
                    battery: StatusBar::from_battery_percent(battery.percent, false)
                        .or(Some(BatteryMarker::Critical)),
                    wifi: None,
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
