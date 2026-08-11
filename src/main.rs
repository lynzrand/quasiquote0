//! Minimal Quote/0 status firmware.

use esp_idf_hal::peripherals::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use quasiquote0::bluetooth::Bluetooth;
use quasiquote0::display::{Epd, EpdPins, Frame};
use quasiquote0::jsonrpc;
use quasiquote0::usb_serial::UsbSerial;
use quasiquote0::yamux::{Mode, Yamux};
use quasiquote0::{battery, link_patches, status_screen, wifi::WifiStation};

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

    // Credentials and connection policy are intentionally not included yet.
    let wifi = EspSystemEventLoop::take()
        .ok()
        .and_then(|event_loop| {
            EspDefaultNvsPartition::take()
                .ok()
                .map(|nvs| (event_loop, nvs))
        })
        .and_then(|(event_loop, nvs)| WifiStation::new(modem, event_loop, Some(nvs)).ok());
    let bluetooth = Bluetooth::start().ok();

    let mut frame = Frame::white();
    let wifi_up = wifi
        .as_ref()
        .and_then(|wifi| wifi.is_up().ok())
        .unwrap_or(false);
    let bluetooth_ready = bluetooth.as_ref().is_some_and(Bluetooth::is_ready);
    status_screen::render(&mut frame, wifi_up, bluetooth_ready, battery::read());
    display
        .display(&frame)
        .expect("initial status display refresh failed");

    let usb = UsbSerial::new(usb_serial, pins.gpio18, pins.gpio19).expect("USB setup failed");
    let yamux = Yamux::start(usb, Mode::Server).expect("Yamux driver failed");
    jsonrpc::serve(yamux.control_stream()).expect("stream-0 JSON-RPC failed");
}
