//! Recovery-first clock bring-up. No networking, BLE, filesystem, or frame-client setup.

use esp_idf_hal::{delay::FreeRtos, peripherals::Peripherals};
use quasiquote0::clock_face::{self, ClockFace};
use quasiquote0::display::{Epd, EpdPins};
use quasiquote0::frame::Projection;
use quasiquote0::jsonrpc::{self, Error, Handler, Request};
use quasiquote0::usb_serial::UsbSerial;
use quasiquote0::wall_clock::unix_seconds;
use quasiquote0::yamux::{Mode, Yamux};
use quasiquote0::{battery, power};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct DisplayState {
    displayed: Option<ClockFace>,
    refreshes: u32,
    error: Option<&'static str>,
    wake_error: Option<&'static str>,
}

struct ClockRpc {
    display: Arc<Mutex<DisplayState>>,
    reboot: bool,
}

impl Handler for ClockRpc {
    fn handle(&mut self, request: Request) -> Result<Value, Error> {
        match request.method.as_str() {
            "system.ping" => Ok(json!({"service":"quasiquote0", "mode":"clock"})),
            "device.hello" => Ok(json!({"protocol":1, "firmware":env!("CARGO_PKG_VERSION"),
                "device":{"model":"Quote/0"}, "mode":"clock"})),
            "clock.set" => {
                let seconds = request
                    .params
                    .get("unix_seconds")
                    .and_then(Value::as_i64)
                    .filter(|v| clock_face::valid_time(*v))
                    .ok_or_else(|| {
                        Error::invalid_params("unix_seconds must be between 2020 and 2100")
                    })?;
                quasiquote0::wall_clock::set(seconds).map_err(Error::invalid_params)?;
                Ok(json!({"accepted":true, "unix_seconds":seconds,
                    "utc_offset_minutes":clock_face::UTC_OFFSET_MINUTES}))
            }
            "clock.get" | "device.status" => {
                let seconds = unix_seconds();
                let guard = power::guard();
                let state = self.display.lock().unwrap();
                Ok(json!({"mode":"clock", "unix_seconds":seconds,
                    "time":ClockFace::at(seconds).text(),
                    "synchronized":clock_face::valid_time(seconds),
                    "utc_offset_minutes":clock_face::UTC_OFFSET_MINUTES,
                    "display":{"time":state.displayed.map(ClockFace::text),
                        "refreshes":state.refreshes, "error":state.error},
                    "power":{"vbus_present":guard.vbus_present,
                        "usb_host_connected":guard.usb_host_connected,
                        "must_stay_awake":guard.must_stay_awake(),
                        "uptime_ms":guard.uptime_ms, "timer_wake":guard.timer_wake,
                        "wake_sources_armed":state.wake_error.is_none(),
                        "wake_error":state.wake_error}}))
            }
            "device.reboot" => {
                self.reboot = true;
                Ok(json!({"rebooting":true}))
            }
            _ => Err(Error::method_not_found()),
        }
    }

    fn after_response(&mut self) {
        if self.reboot {
            FreeRtos::delay_ms(100);
            unsafe {
                esp_idf_sys::esp_restart();
            }
        }
    }
}

fn main() {
    quasiquote0::link_patches();
    let peripherals = Peripherals::take().expect("peripherals already taken");
    let pins = peripherals.pins;
    let usb =
        UsbSerial::new(peripherals.usb_serial, pins.gpio18, pins.gpio19).expect("USB setup failed");
    let yamux = Yamux::start(usb, Mode::Server).expect("USB transport setup failed");
    let state = Arc::new(Mutex::new(DisplayState {
        wake_error: power::arm_wake_sources(60).err(),
        ..DisplayState::default()
    }));
    let mut rpc = ClockRpc {
        display: state.clone(),
        reboot: false,
    };
    std::thread::Builder::new()
        .name("clock-usb".into())
        .stack_size(8 * 1024)
        .spawn(move || {
            let _ = jsonrpc::serve(yamux.control_stream(), &mut rpc);
        })
        .expect("USB control task failed");
    let mut display = Epd::new(
        peripherals.spi2,
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
    if display.is_none() {
        state.lock().unwrap().error = Some("display initialization failed");
    }
    let _ = quasiquote0::ota::confirm_running_image();

    loop {
        let seconds = unix_seconds();
        let face = ClockFace::at(seconds);
        let battery = battery::read();
        let critical = !battery.usb_present
            && battery
                .millivolts
                .is_some_and(|mv| mv < power::CRITICAL_BATTERY_MV);
        if !critical && state.lock().unwrap().displayed != Some(face) {
            if let Some(display) = display.as_mut() {
                let frame = clock_face::render(face).project(Projection::Clockwise);
                let result = display.display(&frame);
                let mut state = state.lock().unwrap();
                state.error = result.err();
                if state.error.is_none() {
                    state.displayed = Some(face);
                    state.refreshes += 1;
                }
            }
        }
        let sleep = if critical {
            power::CRITICAL_SLEEP_SECONDS
        } else {
            clock_face::seconds_until_next_minute(unix_seconds())
        };
        if let Err(error) = power::try_deep_sleep(sleep) {
            state.lock().unwrap().wake_error = Some(error);
        }
        FreeRtos::delay_ms(250);
    }
}
