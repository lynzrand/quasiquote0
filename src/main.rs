//! Quote/0 hardware wiring and recovery-first startup.
use esp_idf_hal::peripherals::Peripherals;
use quasiquote0::{
    app::App,
    appliance::Appliance,
    display::{Epd, EpdPins},
    link_patches,
    network::Network,
    usb_serial::UsbSerial,
};
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
    let display = Epd::new(
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
    Appliance::new(app, display, Network::new(modem)).run();
}
