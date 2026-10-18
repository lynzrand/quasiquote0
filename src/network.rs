//! One radio owner: persistent station/setup on USB; bounded pull windows on battery.
use crate::{
    app::App,
    config::{Config, WifiConfig},
    network_policy::{self, LinkMode},
    rtc_schedule,
    wifi::WifiStation,
};
use esp_idf_hal::modem::Modem;
use esp_idf_svc::{eventloop::EspSystemEventLoop, nvs::EspDefaultNvsPartition, sntp::EspSntp};
use std::sync::Arc;
pub struct Network<'d> {
    modem: Option<Modem<'d>>,
    wifi: Option<WifiStation<'d>>,
    mode: LinkMode,
    applied: WifiConfig,
    retry_at: u64,
    started: u64,
    ntp_started: u64,
    ntp_retry_at: u64,
    ntp_error: Option<String>,
    ntp: Option<EspSntp<'static>>,
}
#[derive(Default)]
pub struct Link {
    pub ready: bool,
    pub busy: bool,
    pub failed: bool,
    pub time_waiting: bool,
}
impl<'d> Network<'d> {
    pub fn new(modem: Modem<'d>) -> Self {
        Self {
            modem: Some(modem),
            wifi: None,
            mode: LinkMode::Off,
            applied: WifiConfig::default(),
            retry_at: 0,
            started: 0,
            ntp_started: 0,
            ntp_retry_at: 0,
            ntp_error: None,
            ntp: None,
        }
    }
    pub fn tick(&mut self, app: &Arc<App>, config: &Config, usb: bool, pull_due: bool) -> Link {
        let now = rtc_schedule::now_ms();
        let desired = network_policy::desired(usb, !config.wifi.ssid.is_empty(), pull_due);
        if desired == LinkMode::Off {
            if self.mode != LinkMode::Off {
                self.stop();
            }
            let mut status = app.network.lock().unwrap();
            status.mode = "off";
            status.connected = false;
            status.ip = None;
            status.url = None;
            return Link::default();
        }
        if self.applied != config.wifi {
            if now < *app.wifi_change_after_ms.lock().unwrap() {
                return Link {
                    busy: true,
                    ..Default::default()
                };
            }
            self.retry_at = 0;
            self.stop();
            self.applied = config.wifi.clone();
        }
        if now < self.retry_at {
            return Link::default();
        }
        if self.wifi.is_none() {
            let result = (|| {
                let modem = self
                    .modem
                    .take()
                    .ok_or("Wi-Fi initialization needs a reboot".to_owned())?;
                let events = EspSystemEventLoop::take().map_err(|e| e.to_string())?;
                let nvs = EspDefaultNvsPartition::take().map_err(|e| e.to_string())?;
                WifiStation::new(modem, events, Some(nvs)).map_err(|e| e.to_string())
            })();
            match result {
                Ok(wifi) => self.wifi = Some(wifi),
                Err(error) => return self.fail(app, now, error),
            }
        }
        if self.mode != desired {
            self.stop();
            let wifi = self.wifi.as_mut().unwrap();
            let result = match desired {
                LinkMode::Setup => wifi.configure_setup(&app.setup_ssid, &app.access_token[..12]),
                LinkMode::Station => wifi.configure(&config.wifi.ssid, &config.wifi.password),
                LinkMode::Off => unreachable!(),
            }
            .and_then(|()| wifi.start().map_err(|e| e.to_string()))
            .and_then(|()| {
                if desired == LinkMode::Station {
                    wifi.connect().map_err(|e| e.to_string())
                } else {
                    Ok(())
                }
            });
            if let Err(error) = result {
                return self.fail(app, now, error);
            }
            self.mode = desired;
            self.applied = config.wifi.clone();
            self.started = now;
        }
        let setup = self.mode == LinkMode::Setup;
        let ready = setup || self.wifi.as_ref().unwrap().is_up().unwrap_or(false);
        if !ready && now.saturating_sub(self.started) > 15_000 {
            return self.fail(
                app,
                now,
                "Wi-Fi connection timed out; USB setup remains available".into(),
            );
        }
        {
            let mut status = app.network.lock().unwrap();
            status.mode = if setup { "setup" } else { "station" };
            status.connected = ready;
            status.ip = if ready {
                self.wifi.as_ref().unwrap().ip(setup)
            } else {
                None
            };
            status.url = status.ip.as_ref().map(|ip| format!("http://{ip}/"));
            status.error = self.ntp_error.clone();
        }
        if ready && !setup && self.ntp.is_none() && now >= self.ntp_retry_at {
            let shared = app.clone();
            match EspSntp::new_with_callback(&Default::default(), move |time| {
                shared.network.lock().unwrap().last_ntp_unix_seconds = Some(time.as_secs() as i64);
            }) {
                Ok(ntp) => {
                    self.ntp_error = None;
                    self.ntp = Some(ntp);
                    self.ntp_started = now;
                }
                Err(error) => {
                    self.ntp_error = Some(format!("NTP: {error}"));
                    self.ntp_retry_at = now.saturating_add(30_000);
                    app.network.lock().unwrap().error = self.ntp_error.clone();
                    self.ntp_started = now.saturating_sub(12_001);
                }
            }
        }
        let unknown_time = !crate::clock_face::valid_time(crate::wall_clock::unix_seconds());
        Link {
            ready: ready && !setup,
            busy: !ready,
            time_waiting: ready
                && !setup
                && unknown_time
                && self.ntp.is_some()
                && now.saturating_sub(self.ntp_started) < 12_000,
            failed: false,
        }
    }
    fn fail(&mut self, app: &App, now: u64, error: String) -> Link {
        self.stop();
        self.retry_at = now.saturating_add(30_000);
        let mut status = app.network.lock().unwrap();
        status.mode = "off";
        status.connected = false;
        status.ip = None;
        status.url = None;
        status.error = Some(error);
        Link {
            failed: true,
            ..Default::default()
        }
    }
    fn stop(&mut self) {
        self.ntp.take();
        self.ntp_error = None;
        self.ntp_retry_at = 0;
        if let Some(wifi) = &mut self.wifi {
            let _ = wifi.stop();
        }
        self.mode = LinkMode::Off;
    }
}
