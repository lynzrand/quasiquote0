//! One owner coordinates networking, telemetry, rendering, and battery sleep.
mod render;

use crate::{
    activity::OPERATIONS,
    app::App,
    battery::{self, BatteryStatus},
    bluetooth::Bluetooth,
    bthome::BthomeAdvertisement,
    clock_face,
    config::Config,
    display::Epd,
    management,
    network::{Link, Network},
    power, pull,
    pull_schedule::PullSchedule,
    rtc_schedule,
    status_bar::{BatteryMarker, StatusBar, WifiMarker},
    wall_clock,
};
use esp_idf_hal::delay::FreeRtos;
use esp_idf_svc::http::server::EspHttpServer;
use render::Renderer;
use std::sync::{atomic::Ordering, Arc};

const LOOP_DELAY_MS: u32 = 250;
const SERVER_RETRY_MS: u64 = 30_000;
const BOOT_CONFIRM_AFTER_MS: u64 = 10_000;

pub struct Appliance<'d> {
    app: Arc<App>,
    renderer: Renderer<'d>,
    network: Network<'d>,
    bluetooth: Option<Bluetooth>,
    server: Option<EspHttpServer<'static>>,
    server_retry_at: u64,
    schedule: PullSchedule,
    boot_confirmed: bool,
}

impl<'d> Appliance<'d> {
    pub fn new(app: Arc<App>, display: Option<Epd<'d>>, network: Network<'d>) -> Self {
        let bluetooth = Bluetooth::start().ok();
        let renderer = Renderer::new(display, app.device.as_deref());
        Self {
            app,
            renderer,
            network,
            bluetooth,
            server: None,
            server_retry_at: 0,
            schedule: rtc_schedule::load(),
            boot_confirmed: false,
        }
    }

    pub fn run(mut self) -> ! {
        loop {
            let delay = self.tick();
            FreeRtos::delay_ms(delay);
        }
    }

    fn tick(&mut self) -> u32 {
        let battery = battery::read();
        let usb = battery.usb_present;
        let config = self.app.config();
        if !usb {
            self.server.take();
        }
        self.restart_if_requested();
        if !usb
            && battery
                .millivolts
                .is_some_and(|mv| mv < power::CRITICAL_BATTERY_MV)
        {
            self.network.tick(&self.app, &config, false, false);
            self.sleep(power::CRITICAL_SLEEP_SECONDS);
            return 1_000;
        }
        let due = config.pull.enabled
            && self.schedule.is_due(
                rtc_schedule::now_ms(),
                self.app.pull_requested.load(Ordering::Acquire),
            );
        let link = self.network.tick(&self.app, &config, usb, due);
        self.start_browser_if_ready(usb);
        self.pull_if_ready(&config, &link, usb, due);
        self.refresh_telemetry(battery, &link);
        self.confirm_boot_if_ready();
        self.app.main_stack_free.store(
            unsafe { esp_idf_sys::uxTaskGetStackHighWaterMark(core::ptr::null_mut()) },
            Ordering::Relaxed,
        );
        // Finish connecting and initial NTP before a battery pull can sleep.
        if !link.busy && !link.time_waiting {
            self.sleep(self.sleep_seconds(&config));
        } else {
            rtc_schedule::save(&self.schedule);
        }
        LOOP_DELAY_MS
    }

    fn restart_if_requested(&self) {
        if self.app.reboot_requested.load(Ordering::Acquire) {
            if let Some(_operation) = OPERATIONS.try_acquire() {
                FreeRtos::delay_ms(1_000);
                unsafe {
                    esp_idf_sys::esp_restart();
                }
            }
        }
    }

    fn start_browser_if_ready(&mut self, usb: bool) {
        if !usb
            || self.server.is_some()
            || rtc_schedule::now_ms() < self.server_retry_at
            || !self.app.network.lock().unwrap().connected
        {
            return;
        }
        match management::start(self.app.clone()) {
            Ok(server) => self.server = Some(server),
            Err(error) => {
                self.app.network.lock().unwrap().error = Some(format!("Browser service: {error}"));
                self.server_retry_at = rtc_schedule::now_ms().saturating_add(SERVER_RETRY_MS);
            }
        }
    }

    fn pull_if_ready(&mut self, config: &Config, link: &Link, usb: bool, due: bool) {
        if !due {
            return;
        }
        if link.failed {
            self.pull_failed("Wi-Fi connection failed".into());
            return;
        }
        if !link.ready || link.time_waiting {
            return;
        }
        let Some(_operation) = OPERATIONS.try_acquire() else {
            return;
        };
        // Consume before work so a later request remains pending.
        self.app.pull_requested.store(false, Ordering::Release);
        let result = if config.pull.url.starts_with("https://")
            && !clock_face::valid_time(wall_clock::unix_seconds())
        {
            Err("NTP has not established time for HTTPS certificate validation".into())
        } else {
            self.app
                .device
                .as_ref()
                .ok_or_else(|| "Storage recovery required".to_owned())
                .and_then(|device| pull::update(device))
        };
        match result {
            Ok(()) => self
                .schedule
                .succeeded(rtc_schedule::now_ms(), config.pull.interval_seconds),
            Err(error) => self.pull_failed(error),
        }
        if !usb {
            self.network.tick(&self.app, config, false, false);
        }
    }

    fn pull_failed(&mut self, error: String) {
        self.app.pull_requested.store(false, Ordering::Release);
        self.schedule.failed(rtc_schedule::now_ms());
        if let Some(device) = &self.app.device {
            device.lock().unwrap().record_pull_error(error);
        }
    }

    fn refresh_telemetry(&mut self, battery: BatteryStatus, link: &Link) {
        let connected = {
            let network = self.app.network.lock().unwrap();
            network.mode == "station" && network.connected
        };
        if let Some(bluetooth) = &self.bluetooth {
            bluetooth.advertise(&BthomeAdvertisement {
                battery_percent: battery.percent.unwrap_or_default() as u8,
                battery_mv: battery.millivolts.unwrap_or_default() as u16,
                battery_low: battery.percent.is_some_and(|percent| percent < 20),
                charging: battery.usb_present,
                wifi_connected: connected,
                usb_present: battery.usb_present,
            });
        }
        let advertising = self
            .bluetooth
            .as_ref()
            .is_some_and(Bluetooth::is_advertising);
        self.app
            .bluetooth_advertising
            .store(advertising, Ordering::Relaxed);
        let wifi = if connected {
            WifiMarker::Connected
        } else if self.schedule.consecutive_failures() > 0 {
            WifiMarker::Failed
        } else if link.busy {
            WifiMarker::Connecting
        } else {
            WifiMarker::Disconnected
        };
        if let Some(device) = &self.app.device {
            self.renderer.refresh(
                device,
                StatusBar {
                    battery: if battery
                        .millivolts
                        .is_some_and(|mv| mv < power::CRITICAL_BATTERY_MV)
                    {
                        Some(BatteryMarker::Critical)
                    } else {
                        StatusBar::from_battery_percent(battery.percent, false)
                    },
                    wifi: Some(wifi),
                    usb_present: battery.usb_present,
                    bluetooth_present: advertising,
                    unix_seconds: Some(wall_clock::unix_seconds()),
                },
            );
        }
    }

    fn confirm_boot_if_ready(&mut self) {
        if self.boot_confirmed || power::guard().uptime_ms < BOOT_CONFIRM_AFTER_MS {
            return;
        }
        if let Some(device) = &self.app.device {
            let mut device = device.lock().unwrap();
            if device.display_refreshes > 0
                && device.display_error.is_none()
                && self.app.recovery_error.is_none()
            {
                self.boot_confirmed = device.confirm_ota().is_ok();
            }
        }
    }

    fn sleep_seconds(&self, config: &Config) -> u64 {
        let heartbeat = if config.display.status_bar {
            clock_face::seconds_until_next_minute(wall_clock::unix_seconds())
        } else {
            power::IDLE_SLEEP_SECONDS
        };
        if config.pull.enabled {
            heartbeat.min(self.schedule.seconds_until_due(rtc_schedule::now_ms()))
        } else {
            heartbeat
        }
    }

    fn sleep(&self, seconds: u64) {
        rtc_schedule::save(&self.schedule);
        let _ = power::try_deep_sleep(seconds);
    }
}
